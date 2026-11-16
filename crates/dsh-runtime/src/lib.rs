//! Drives one DeepSeek Harness kernel process over its SDK wire protocol.
//!
//! The kernel speaks JSON-RPC 2.0, one message per `\n`-terminated line, over
//! stdio. [`protocol`] owns the vocabulary and frame classification;
//! [`process`] owns the child process and its three pipes.
//!
//! Typical use:
//!
//! ```no_run
//! # async fn demo() -> Result<(), dsh_runtime::RuntimeError> {
//! use std::time::Duration;
//! use dsh_runtime::{KernelClient, KernelConfig};
//!
//! let mut config = KernelConfig {
//!     node: "node".into(),
//!     entry: "/repo/kernel/node_modules/@deepseek-ai/dsh/lib/bin.js".into(),
//!     profile: "sdk".into(),
//!     patches: vec![],
//!     home: "/repo/.dsh-home".into(),
//!     cwd: "/repo".into(),
//!     env: vec![("DEEPSEEK_API_KEY".into(), "...".into())],
//! };
//! let client = KernelClient::spawn(config).await?;
//! let mut events = client.subscribe();
//! client
//!     .initialize(
//!         serde_json::json!({
//!             "cwd": "/repo",
//!             "provider": "deepseek-official",
//!             "model": "deepseek-v4.1-flash",
//!             "approvalRequests": true,
//!         }),
//!         Duration::from_secs(10),
//!     )
//!     .await?;
//! # Ok(())
//! # }
//! ```

pub mod pool;
pub mod process;
pub mod prompt;
pub mod protocol;

pub use pool::{KernelPool, RestartError, RestartPolicy, Route};
pub use process::{
    ExitStatus, KernelClient, KernelConfig, LIVE_CHUNK_METHOD, LIVE_PREFIX, RuntimeError,
    kernel_entry,
};
pub use protocol::{ApprovalOutcome, FrameId, Incoming, ProtocolError, ResponsePayload, RpcError};
