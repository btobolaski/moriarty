use super::*;

fn settled_response(response: Result<ServerResult, McpError>) -> Result<Value, McpError> {
    settle_tool_response(response, Err(miette::miette!("cleanup")))
        .map(|result| to_value(result).unwrap())
}

#[test]
fn tool_result_retains_content_on_cleanup_failure() {
    for initial in [CallToolResult::success, CallToolResult::error] {
        let original = initial(vec![ContentBlock::text("primary")]);
        let result = settled_response(Ok(ServerResult::CallToolResult(original))).unwrap();
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "primary");
        assert_eq!(result["content"][1]["text"], "Settlement: cleanup");
    }
}

#[test]
fn request_error_retains_both_causes() {
    let original = Err(McpError::invalid_request("primary", None));
    let error = settled_response(original).unwrap_err();
    assert!(error.message.contains("primary") && error.message.contains("cleanup"));
}

#[tokio::test]
async fn request_cancellation_is_local_and_drops_its_work() {
    for modern in [false, true] {
        for cancelled in [0, 1] {
            let server = PendingTools::default();
            let mut client = TestClient::connect(server.clone(), modern).await;
            server.start_call(&mut client, 0, 11).await;
            server.start_call(&mut client, 1, 22).await;
            let ids = [11, 22];
            client
                .send(json!({
                    "jsonrpc":"2.0", "method":"notifications/cancelled",
                    "params":{"requestId":ids[cancelled], "reason":"test cancellation"}
                }))
                .await;
            timeout(TEST_TIMEOUT, server.done[cancelled].notified())
                .await
                .unwrap();
            assert!(server.dropped[cancelled].load(Ordering::SeqCst));
            let scope = server.scopes[cancelled].lock().unwrap().clone().unwrap();
            assert_scope_rejects_admission(&scope).await;
            let other = 1 - cancelled;
            assert!(
                !server.dropped[other].load(Ordering::SeqCst),
                "another request must remain active"
            );
            server.release[other].notify_one();
            loop {
                let response = client.response().await;
                if response["id"] == ids[other] {
                    assert_result_shape(&response, modern, false);
                    assert_eq!(response["result"]["content"][0]["text"], "released");
                    break;
                }
                assert_eq!(response["id"], ids[cancelled]);
                assert!(response.get("error").is_some());
            }
            client.close().await;
        }
    }
}

#[derive(Default)]
struct GatedOwner {
    entered: Notify,
    cancelled: Notify,
    release: Notify,
    completed: Notify,
    token: Mutex<Option<CancellationToken>>,
}

fn gated_owners(cleanup_failure: bool) -> (Arc<Scope>, [Arc<GatedOwner>; 2]) {
    let owners: [Arc<GatedOwner>; 2] = Default::default();
    let scope = scope_with_operations({
        let owners = owners.clone();
        move |spec, token| {
            let index = usize::from(spec.program == "second");
            let owner = owners[index].clone();
            *owner.token.lock().unwrap() = Some(token.clone());
            async move {
                owner.entered.notify_one();
                token.cancelled().await;
                owner.cancelled.notify_one();
                owner.release.notified().await;
                owner.completed.notify_one();
                if cleanup_failure && index == 1 {
                    Err(ProcessError::CleanupUnconfirmed {
                        state: Box::<PartialState>::default(),
                    }
                    .into())
                } else {
                    Ok(test_report(Completion::Cancelled(ExitStatus::from_raw(0))))
                }
            }
        }
    });
    (scope, owners)
}

#[tokio::test]
async fn cancellation_and_disconnect_await_owners_and_retain_cleanup_after_response_loss() {
    for modern in [false, true] {
        for cleanup_failure in [false, true] {
            for cancelled_request in [None, Some(0), Some(1)] {
                let (scope, owners) = gated_owners(cleanup_failure);
                let server = PendingTools {
                    owned_work: true,
                    ..Default::default()
                };
                let mut client =
                    TestClient::start_in_scope(server.clone(), pending(), scope.clone(), modern)
                        .await;
                if !modern {
                    initialize_client(&mut client, "2025-11-25").await;
                }
                server.start_call(&mut client, 0, 11).await;
                server.start_call(&mut client, 1, 22).await;
                for owner in &owners {
                    timeout(TEST_TIMEOUT, owner.entered.notified())
                        .await
                        .unwrap();
                }
                if let Some(index) = cancelled_request {
                    client
                        .send(json!({
                            "jsonrpc":"2.0", "method":"notifications/cancelled",
                            "params":{"requestId":([11, 22][index])}
                        }))
                        .await;
                    timeout(TEST_TIMEOUT, server.done[index].notified())
                        .await
                        .unwrap();
                    timeout(TEST_TIMEOUT, owners[index].cancelled.notified())
                        .await
                        .unwrap();
                    assert!(
                        !owners[1 - index]
                            .token
                            .lock()
                            .unwrap()
                            .as_ref()
                            .unwrap()
                            .is_cancelled(),
                        "request cancellation reached an unrelated process owner"
                    );
                    let request_scope = server.scopes[index].lock().unwrap().clone().unwrap();
                    assert_scope_rejects_admission(&request_scope).await;
                }
                let TestClient {
                    reader,
                    writer,
                    mut server_task,
                    ..
                } = client;
                drop(reader);
                drop(writer);
                // Both owners must receive cancellation before either is permitted to settle.
                for (index, owner) in owners.iter().enumerate() {
                    if Some(index) != cancelled_request {
                        timeout(TEST_TIMEOUT, owner.cancelled.notified())
                            .await
                            .unwrap();
                    }
                }
                assert_scope_rejects_admission(&scope).await;
                for owner in &owners {
                    poll_fn(|context| {
                        assert!(
                            Pin::new(&mut server_task).poll(context).is_pending(),
                            "host returned before every owner settled"
                        );
                        Poll::Ready(())
                    })
                    .await;
                    owner.release.notify_one();
                    timeout(TEST_TIMEOUT, owner.completed.notified())
                        .await
                        .unwrap();
                }
                let outcome = timeout(TEST_TIMEOUT, server_task).await.unwrap().unwrap();
                if cleanup_failure {
                    let error = outcome.unwrap_err();
                    assert!(
                        error
                            .to_string()
                            .contains("cleanup or capture settlement unconfirmed"),
                        "{error}"
                    );
                    assert!(
                        scope.shutdown().await.is_err(),
                        "cleanup failure must remain host-owned"
                    );
                } else if let Err(error) = outcome {
                    assert!(
                        error.to_string().contains("MCP transport failed"),
                        "{error}"
                    );
                }
                assert!(
                    server
                        .dropped
                        .iter()
                        .all(|done| done.load(Ordering::SeqCst))
                );
            }
        }
    }
}

#[tokio::test]
async fn disconnect_settles_every_dropped_handler_before_return() {
    for modern in [false, true] {
        for graceful_eof in [true, false] {
            let server = PendingTools::default();
            let mut client = TestClient::connect(server.clone(), modern).await;
            server.start_call(&mut client, 0, 11).await;
            server.start_call(&mut client, 1, 22).await;
            let TestClient {
                modern: _,
                reader,
                mut writer,
                server_task,
            } = client;
            let reader = if graceful_eof {
                writer.shutdown().await.unwrap();
                Some(reader)
            } else {
                drop(reader);
                None
            };
            drop(writer);
            let outcome = timeout(TEST_TIMEOUT, server_task).await.unwrap().unwrap();
            if graceful_eof {
                outcome.unwrap();
            } else if let Err(error) = outcome {
                assert!(
                    error.to_string().contains("MCP transport failed"),
                    "{error}"
                );
            }
            drop(reader);
            assert!(
                server
                    .dropped
                    .iter()
                    .all(|done| done.load(Ordering::SeqCst))
            );
        }
    }
}

async fn assert_host_failure(
    task: JoinHandle<miette::Result<()>>,
    causes: &[&str],
    server: &PendingTools,
) {
    let Err(error) = timeout(TEST_TIMEOUT, task).await.unwrap().unwrap() else {
        panic!("host must fail")
    };
    for cause in causes {
        assert!(error.to_string().contains(cause), "{error}");
    }
    assert!(server.dropped[0].load(Ordering::SeqCst));
}

#[tokio::test]
async fn host_signal_preserves_cause_and_awaits_request_settlement() {
    let server = PendingTools::default();
    let (trigger, signal) = oneshot::channel();
    let mut client = TestClient::start_until(server.clone(), async {
        signal.await.unwrap();
        Ok("SIGTERM")
    })
    .await;
    initialize_client(&mut client, "2025-11-25").await;
    server.start_call(&mut client, 0, 11).await;
    client.send(json!({"jsonrpc":"2.0", "id":12, "method":"tools/call", "params":{"name":"large", "arguments":{}}})).await;
    let prefix = timeout(TEST_TIMEOUT, client.reader.fill_buf())
        .await
        .unwrap()
        .unwrap();
    assert!(!prefix.is_empty());
    // Keep response reads stopped while a second request still owns work.
    trigger.send(()).unwrap();
    assert_host_failure(
        client.server_task,
        &["SIGTERM", "MCP transport failed"],
        &server,
    )
    .await;
}

async fn failure_client<R, W>(
    server: &PendingTools,
    wrap: impl FnOnce((ReadHalf<DuplexStream>, WriteHalf<DuplexStream>), Arc<AtomicBool>) -> (R, W),
) -> (TestClient, Arc<AtomicBool>)
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let fail = Arc::new(AtomicBool::new(false));
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(serve_until(
        server.clone(),
        wrap(split(server_io), fail.clone()),
        pending(),
        Scope::new(),
    ));
    let (reader, writer) = split(client_io);
    let mut client = TestClient {
        modern: false,
        reader: BufReader::new(reader),
        writer,
        server_task,
    };
    initialize_client(&mut client, "2025-11-25").await;
    server.start_call(&mut client, 0, 11).await;
    (client, fail)
}

struct FailingRead<R> {
    inner: R,
    fail: Arc<AtomicBool>,
}
impl<R: AsyncRead + Unpin> AsyncRead for FailingRead<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.fail.load(Ordering::SeqCst) {
            return Poll::Ready(Err(io::Error::other("injected transport failure")));
        }
        Pin::new(&mut this.inner).poll_read(context, buffer)
    }
}

#[tokio::test]
async fn transport_failure_is_not_successful_eof_and_settles_handlers() {
    let server = PendingTools::default();
    let (mut client, fail) = failure_client(&server, |(reader, writer), fail| {
        (
            FailingRead {
                inner: reader,
                fail,
            },
            writer,
        )
    })
    .await;
    fail.store(true, Ordering::SeqCst);
    client.writer.write_all(b"\n").await.unwrap();
    assert_host_failure(client.server_task, &["injected transport failure"], &server).await;
}

#[tokio::test]
async fn guarded_ping_backpressure_does_not_hide_eof_or_owner_settlement() {
    for cleanup_failure in [false, true] {
        let (scope, owners) = gated_owners(cleanup_failure);
        let server = PendingTools {
            owned_work: true,
            ..Default::default()
        };
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let (reader, writer) = split(server_io);
        let (writer, blocked, _resume) = GatedWrite::new(writer);
        let server_task = tokio::spawn(serve_until(
            server.clone(),
            (reader, writer),
            pending(),
            scope.clone(),
        ));
        let (reader, writer) = split(client_io);
        let mut client = TestClient {
            modern: true,
            reader: BufReader::new(reader),
            writer,
            server_task,
        };
        server.start_call(&mut client, 1, 11).await;
        timeout(TEST_TIMEOUT, owners[1].entered.notified())
            .await
            .unwrap();
        client
            .send_request(json!({
                "jsonrpc":"2.0", "id":12, "method":"tools/call",
                "params":{"name":"large", "arguments":{}}
            }))
            .await;
        timeout(TEST_TIMEOUT, blocked.notified()).await.unwrap();
        client
            .send_request(json!({"jsonrpc":"2.0", "id":13, "method":"ping", "params":{}}))
            .await;
        client.writer.shutdown().await.unwrap();

        // Output remains blocked and the response reader stays alive but unpolled.
        timeout(TEST_TIMEOUT, owners[1].cancelled.notified())
            .await
            .unwrap();
        assert_scope_rejects_admission(&scope).await;
        let request_scope = server.scopes[1].lock().unwrap().clone().unwrap();
        assert_scope_rejects_admission(&request_scope).await;
        poll_fn(|context| {
            assert!(
                Pin::new(&mut client.server_task).poll(context).is_pending(),
                "host returned before its process owner settled"
            );
            Poll::Ready(())
        })
        .await;
        owners[1].release.notify_one();
        timeout(TEST_TIMEOUT, owners[1].completed.notified())
            .await
            .unwrap();
        let error = timeout(TEST_TIMEOUT, client.server_task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(
            error.to_string().contains("MCP transport failed"),
            "{error}"
        );
        if cleanup_failure {
            assert!(
                error
                    .to_string()
                    .contains("cleanup or capture settlement unconfirmed"),
                "{error}"
            );
        }
        assert!(server.dropped[1].load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn transport_write_failure_surfaces_and_settles_handlers() {
    for zero in [false, true] {
        let server = PendingTools::default();
        let (mut client, fail) = failure_client(&server, |(reader, writer), fail| {
            (
                reader,
                FailingWrite {
                    inner: writer,
                    fail,
                    zero,
                },
            )
        })
        .await;
        fail.store(true, Ordering::SeqCst);
        client
            .send(json!({"jsonrpc":"2.0", "id":12, "method":"tools/list"}))
            .await;
        let expected = if zero {
            io::Error::from(io::ErrorKind::WriteZero).to_string()
        } else {
            "injected write failure".into()
        };
        let causes = ["MCP transport failed", expected.as_str()];
        assert_host_failure(client.server_task, &causes, &server).await;
    }
}

#[tokio::test]
async fn service_join_failures_preserve_the_cause() {
    for nested in [false, true] {
        let task = tokio::spawn(pending::<()>());
        task.abort();
        let error = task.await.unwrap_err();
        assert!(error.is_cancelled());
        let expected = error.to_string();
        let result = if nested {
            Ok(QuitReason::JoinError(error))
        } else {
            Err(error)
        };
        assert_eq!(service_exit(result).unwrap_err().to_string(), expected);
    }
    service_exit(Ok(QuitReason::Closed)).unwrap();
    service_exit(Ok(QuitReason::Cancelled)).unwrap();
}
