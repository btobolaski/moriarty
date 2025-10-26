use clap::Subcommand;

use git_read_only::GitReadOnly;
use miette::IntoDiagnostic;
use rmcp::{transport::stdio, ServiceExt};

pub mod git_read_only;

#[derive(Debug, Subcommand)]
pub enum McpServers {
    /// runs the git read only server as stdin / stdout server
    GitReadOnly,
}

impl McpServers {
    pub async fn run(self) -> miette::Result<()> {
        match self {
            Self::GitReadOnly => {
                let server = GitReadOnly::default();

                for tool in server.tool_router.list_all() {
                    eprintln!("\n{}: {}", tool.name, tool.description.unwrap_or_default());

                    if let Some(output_schema) = &tool.output_schema {
                        eprintln!(
                            " Output schema: {}",
                            serde_json::to_string_pretty(output_schema).unwrap()
                        );
                    } else {
                        eprintln!(" Output: Unstructured text");
                    }
                }

                let service = server.serve(stdio()).await.into_diagnostic()?;

                service.waiting().await.into_diagnostic()?;

                Ok(())
            }
        }
    }
}
