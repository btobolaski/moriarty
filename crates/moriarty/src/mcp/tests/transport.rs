use super::*;

fn observed<T>(inner: T) -> ObservedIo<T> {
    let service_ct = CancellationToken::new();
    ObservedIo::new(
        inner,
        &Scope::new(),
        &Arc::new(Mutex::new(None)),
        &service_ct,
    )
}

#[tokio::test]
async fn pending_output_wakes_on_service_cancellation_without_peer_reads() {
    let (writer, _unread) = tokio::io::duplex(1);
    let mut writer = observed(writer);
    let cancellation = writer.service_ct.clone();
    let mut write = Box::pin(writer.write_all(b"ab"));
    poll_fn(|context| {
        assert!(write.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    cancellation.cancel();
    let error = timeout(TEST_TIMEOUT, write)
        .await
        .expect("cancelled write must wake without a peer read")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
    assert!(writer.failure.lock().unwrap().is_some());
    let Err(error) = writer
        .execution
        .run(CancellationToken::new(), async {})
        .await
    else {
        panic!("cancelled output must close admission")
    };
    assert!(
        error.to_string().contains("cancelled before admission"),
        "{error}"
    );
    writer.execution.shutdown().await.unwrap();
}

#[tokio::test]
async fn zero_capacity_read_keeps_admission_open() {
    let mut io = observed(tokio::io::empty());
    let mut context = Context::from_waker(Waker::noop());
    let mut buffer = ReadBuf::new(&mut []);
    assert!(matches!(
        Pin::new(&mut io).poll_read(&mut context, &mut buffer),
        Poll::Ready(Ok(()))
    ));
    assert!(!io.service_ct.is_cancelled());
    assert!(io.failure.lock().unwrap().is_none());
    assert_eq!(
        io.execution
            .run(CancellationToken::new(), async { 42 })
            .await
            .unwrap()
            .value,
        42
    );
    io.execution.shutdown().await.unwrap();
}

#[tokio::test]
async fn later_transport_failure_preserves_first_cause() {
    let io = observed(tokio::io::empty());
    for cause in ["first", "later"] {
        let Poll::Ready(Err(error)) = io.observe::<()>(Poll::Ready(Err(io::Error::other(cause))))
        else {
            panic!("must propagate failure")
        };
        assert_eq!(error.to_string(), cause);
    }
    assert_eq!(io.failure.lock().unwrap().as_deref(), Some("first"));
    assert!(io.service_ct.is_cancelled());
    let error = io
        .execution
        .run(CancellationToken::new(), async {})
        .await
        .err()
        .expect("closed admission must fail");
    assert!(
        error.to_string().contains("cancelled before admission"),
        "{error}"
    );
    io.execution.shutdown().await.unwrap();
}

#[tokio::test]
async fn guarded_ping_rejection_survives_receive_cancellation_exactly_once() {
    let (input, mut peer) = tokio::io::duplex(1024);
    let (output, mut responses) = tokio::io::duplex(1024);
    let (output, blocked, resume) = GatedWrite::new(output);
    let (mut transport, sender) = ping_transport((input, output).into_transport());
    let mut first_response = Box::pin(
        transport.send(from_value(json!({"jsonrpc":"2.0", "id":90, "result":{}})).unwrap()),
    );
    assert_pending_and_drop(first_response.as_mut()).await;
    timeout(TEST_TIMEOUT, blocked.notified()).await.unwrap();
    write_guarded_ping(&mut peer, 2, "2026-07-28").await;
    assert_pending_and_drop(transport.receive()).await;
    resume.cancel();
    timeout(TEST_TIMEOUT, first_response)
        .await
        .unwrap()
        .unwrap();
    // The next writer is a delivery barrier, without polling receive again.
    timeout(
        TEST_TIMEOUT,
        transport.send(from_value(json!({"jsonrpc":"2.0", "id":91, "result":{}})).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    peer.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/list\",\"params\":{}}\n")
        .await
        .unwrap();
    let forwarded = timeout(TEST_TIMEOUT, transport.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(to_value(forwarded).unwrap()["id"], 3);
    close_with_sender(transport, sender).await;
    let mut output = String::new();
    timeout(TEST_TIMEOUT, responses.read_to_string(&mut output))
        .await
        .unwrap()
        .unwrap();
    let replies: Vec<Value> = output.lines().map(|line| from_str(line).unwrap()).collect();
    assert_eq!(
        replies
            .iter()
            .map(|reply| reply["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(90), json!(2), json!(91)],
        "{replies:?}"
    );
    assert_eq!(replies[1]["error"]["code"], -32601);
    assert!(replies[1].get("result").is_none());
}

fn ping_transport<T: Transport<RoleServer>>(
    inner: T,
) -> (ProtocolGuardTransport<T>, JoinHandle<()>) {
    ProtocolGuardTransport::new(
        inner,
        &Scope::new(),
        &Arc::new(Mutex::new(None)),
        &CancellationToken::new(),
    )
}

// Owned receives are cancelled here; pass a pinned borrow to keep a send alive.
async fn assert_pending_and_drop(future: impl Future) {
    tokio::pin!(future);
    assert!(poll!(future.as_mut()).is_pending());
}

async fn close_with_sender<T: Transport<RoleServer>>(
    mut transport: ProtocolGuardTransport<T>,
    sender: JoinHandle<()>,
) {
    timeout(TEST_TIMEOUT, transport.close())
        .await
        .unwrap()
        .unwrap();
    timeout(TEST_TIMEOUT, sender).await.unwrap().unwrap();
}

async fn write_guarded_ping(peer: &mut DuplexStream, id: usize, version: &str) {
    let message = json!({
        "jsonrpc":"2.0", "id":id, "method":"ping", "params":{"_meta":protocol_meta(version)}
    });
    timeout(
        TEST_TIMEOUT,
        peer.write_all(format!("{message}\n").as_bytes()),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn queued_ping_rejection_failure_closes_admission_and_retains_error() {
    let (input, mut peer) = tokio::io::duplex(1024);
    let fail = Arc::new(AtomicBool::new(false));
    let (output, blocked, resume) = GatedWrite::new(FailingWrite {
        inner: tokio::io::sink(),
        fail: fail.clone(),
        zero: false,
    });
    let (mut transport, sender) = ping_transport((input, output).into_transport());
    write_guarded_ping(&mut peer, 2, "2026-07-28").await;
    assert_pending_and_drop(transport.receive()).await;
    timeout(TEST_TIMEOUT, blocked.notified()).await.unwrap();
    fail.store(true, Ordering::SeqCst);
    resume.cancel();
    timeout(TEST_TIMEOUT, transport.service_ct.cancelled())
        .await
        .unwrap();
    assert_scope_rejects_admission(&transport.execution).await;
    assert!(
        transport
            .failures
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("injected write failure")
    );
    assert!(
        timeout(TEST_TIMEOUT, transport.receive())
            .await
            .unwrap()
            .is_none()
    );
    close_with_sender(transport, sender).await;
}

#[tokio::test]
async fn ping_rejection_queue_is_bounded_and_overflow_fails_closed() {
    let (input, mut peer) = tokio::io::duplex(64 * 1024);
    let (output, blocked, resume) = GatedWrite::new(tokio::io::sink());
    let (mut transport, sender) = ping_transport((input, output).into_transport());
    let mut first_response = Box::pin(
        transport.send(from_value(json!({"jsonrpc":"2.0", "id":90, "result":{}})).unwrap()),
    );
    assert_pending_and_drop(first_response.as_mut()).await;
    timeout(TEST_TIMEOUT, blocked.notified()).await.unwrap();
    // At most one active send plus the bounded queue can remain parked.
    for id in 0..MAX_PENDING_REJECTIONS + 2 {
        write_guarded_ping(&mut peer, id, "2026-07-28").await;
    }
    assert!(
        timeout(TEST_TIMEOUT, transport.receive())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        transport
            .failures
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("MCP rejection queue failed")
    );
    assert!(transport.service_ct.is_cancelled());
    assert_scope_rejects_admission(&transport.execution).await;
    resume.cancel();
    timeout(TEST_TIMEOUT, first_response)
        .await
        .unwrap()
        .unwrap();
    close_with_sender(transport, sender).await;
}

#[tokio::test]
async fn protocol_guard_transport_classifies_requests() {
    for (method, meta, code) in [
        ("server/discover", Some(protocol_meta("2026-07-28")), None),
        ("server/discover", None, None),
        (
            "server/discover",
            Some(json!({
                "io.modelcontextprotocol/protocolVersion":"2099-01-01",
                "io.modelcontextprotocol/clientCapabilities":42
            })),
            Some(-32022),
        ),
        (
            "server/discover",
            Some(json!({"io.modelcontextprotocol/protocolVersion":"2099-01-01"})),
            Some(-32022),
        ),
        ("tools/list", Some(protocol_meta("2026-07-28")), None),
        ("ping", None, None),
        ("ping", Some(json!({})), None),
        ("ping", Some(json!({"progressToken":1})), None),
        ("ping", Some(protocol_meta("2025-11-25")), None),
        ("ping", Some(protocol_meta("2099-01-01")), Some(-32022)),
        ("ping", Some(protocol_meta("2026-07-28")), Some(-32601)),
        (
            "ping",
            Some(json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28"})),
            Some(-32602),
        ),
    ] {
        let params = meta.map_or(json!({}), |meta| json!({"_meta":meta}));
        let message = from_value(json!({
            "jsonrpc":"2.0", "id":1, "method":method, "params":params
        }))
        .unwrap();
        let expected = to_value(&message).unwrap();
        let (inner, mut response) = OneshotTransport::<RoleServer>::new(message);
        let (mut transport, sender) = ping_transport(inner);
        let forwarded = timeout(TEST_TIMEOUT, transport.receive()).await.unwrap();
        if let Some(code) = code {
            assert!(forwarded.is_none(), "{expected}");
            let response = to_value(response.try_recv().unwrap()).unwrap();
            assert_eq!(response["id"], 1, "{response}");
            assert_eq!(response["error"]["code"], code, "{response}");
            assert!(response.get("result").is_none(), "{response}");
        } else {
            assert_eq!(to_value(forwarded.unwrap()).unwrap(), expected);
        }
        close_with_sender(transport, sender).await;
    }
}

#[tokio::test]
async fn ping_rejection_send_failure_closes_transport_before_next_request() {
    let (input, mut peer) = tokio::io::duplex(1024);
    for (id, method) in [(1, "ping"), (2, "tools/list")] {
        let mut line = to_vec(&json!({
            "jsonrpc":"2.0", "id":id, "method":method,
            "params":{"_meta":protocol_meta("2026-07-28")}
        }))
        .unwrap();
        line.push(b'\n');
        peer.write_all(&line).await.unwrap();
    }
    let output = FailingWrite {
        inner: tokio::io::sink(),
        fail: Arc::new(AtomicBool::new(true)),
        zero: false,
    };
    let (mut transport, sender) = ping_transport((input, output).into_transport());
    // A queued ordinary request and an open peer distinguish closure from
    // swallowing the write error and continuing (or waiting for EOF).
    assert!(
        timeout(TEST_TIMEOUT, transport.receive())
            .await
            .unwrap()
            .is_none()
    );
    assert_scope_rejects_admission(&transport.execution).await;
    assert!(
        transport
            .failures
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("injected write failure")
    );
    close_with_sender(transport, sender).await;
}
