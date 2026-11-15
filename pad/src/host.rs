//! Native stand-in for the game's iroh side: accept phones on the controller ALPN and answer
//! each `join` with a `hello`, so the pad page can be tested without the game.
//!
//!     pad-host [seconds]
//!
//! Prints `id <endpoint id>` once the endpoint is up and `joined <line>` per join.

use iroh::Endpoint;
use iroh::endpoint::presets;

#[tokio::main]
async fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let endpoint = Endpoint::builder(presets::N0)
        .relay_mode(hunch_pad::relays())
        .alpns(vec![hunch_pad::ALPN.to_vec()])
        .bind()
        .await
        .expect("iroh endpoint");
    endpoint.online().await;
    println!("id {}", endpoint.id());
    let accept = async {
        while let Some(incoming) = endpoint.accept().await {
            tokio::spawn(async move {
                let Ok(connection) = incoming.await else {
                    return;
                };
                let Ok((mut send, mut recv)) = connection.accept_bi().await else {
                    return;
                };
                let mut chunk = vec![0u8; 4096];
                while let Ok(Some(n)) = recv.read(&mut chunk).await {
                    let text = String::from_utf8_lossy(&chunk[..n]).into_owned();
                    if text.contains(r#""t":"join""#) {
                        println!("joined {}", text.trim());
                        let _ = send.write_all(b"{\"t\":\"hello\",\"id\":1,\"v\":1}\n").await;
                    }
                }
            });
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(seconds), accept).await;
}
