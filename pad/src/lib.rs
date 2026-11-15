//! Adapted from Gobble Grand Prix's pad/src/lib.rs (https://github.com/dsktd/gobble-grand-prix, Unlicense).
//!
//! Phone link to the game over iroh: newline-delimited JSON on one bidirectional
//! stream, the same messages the LAN WebSocket carries. In the browser iroh goes through
//! relays only; the game is found by its endpoint id via iroh's discovery.

use std::str::FromStr;

use async_channel::Receiver;
use async_channel::Sender;
use iroh::Endpoint;
use iroh::EndpointAddr;
use iroh::EndpointId;
use iroh::endpoint::presets;

/// Must match the game.
pub const ALPN: &[u8] = b"hunch/pad/1";

/// Our own relay, then n0's relays (fallback), those without the trailing dot iroh writes in their hostnames: `WebKit` (every iOS
/// browser) rejects the TLS certificate of `use1-1.relay.n0.iroh.link.`. The game uses the
/// same map, so the home relay it publishes is one a browser can dial.
pub fn relays() -> iroh::RelayMode {
    use iroh::defaults::prod;
    let own: iroh::RelayUrl = "https://relay.yak.dsktd.net".parse().expect("relay url");
    let own = iroh::RelayConfig::from(own);
    let urls = [
        prod::NA_EAST_RELAY_HOSTNAME,
        prod::NA_WEST_RELAY_HOSTNAME,
        prod::EU_RELAY_HOSTNAME,
        prod::AP_RELAY_HOSTNAME,
    ]
    .map(|host| {
        let url: iroh::RelayUrl = format!("https://{}", host.trim_end_matches('.'))
            .parse()
            .expect("relay url");
        iroh::RelayConfig::from(url)
    });
    iroh::RelayMode::Custom(iroh::RelayMap::from_iter(std::iter::once(own).chain(urls)))
}

/// An open link: send lines on `outgoing`, read lines from `incoming`.
pub struct Link {
    pub outgoing: Sender<String>,
    pub incoming: Receiver<String>,
    /// Kept so the connection lives as long as the link.
    pub endpoint: Endpoint,
}

/// Connect to a game by endpoint id and pump lines both ways. `spawn` runs the pumps
/// (tokio natively, `spawn_local` in the browser).
pub async fn connect(
    id: &str,
    relay_only: bool,
    spawn: impl Fn(std::pin::Pin<Box<dyn Future<Output = ()> + 'static>>),
) -> Result<Link, String> {
    let id = EndpointId::from_str(id.trim()).map_err(|e| format!("bad game id: {e}"))?;
    let mut builder = Endpoint::builder(presets::N0).relay_mode(relays());
    // Browsers have no UDP and are always relay only
    #[cfg(not(target_arch = "wasm32"))]
    if relay_only {
        builder = builder.clear_ip_transports();
    }
    #[cfg(target_arch = "wasm32")]
    let _ = relay_only;
    let endpoint = builder.bind().await.map_err(|e| format!("iroh: {e}"))?;
    let connection = endpoint
        .connect(EndpointAddr::new(id), ALPN)
        .await
        .map_err(|e| format!("cannot reach the game: {e}"))?;
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| format!("cannot open a stream: {e}"))?;

    let (outgoing, to_game) = async_channel::unbounded::<String>();
    let (from_game, incoming) = async_channel::unbounded::<String>();
    spawn(Box::pin(async move {
        while let Ok(mut line) = to_game.recv().await {
            line.push('\n');
            if send.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
        let _ = send.finish();
    }));
    spawn(Box::pin(async move {
        let mut buf = Vec::new();
        let mut chunk = vec![0u8; 4096];
        let _keep = connection;
        loop {
            let Ok(Some(n)) = recv.read(&mut chunk).await else {
                break;
            };
            buf.extend_from_slice(&chunk[..n]);
            while let Some(end) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=end).collect();
                let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                if from_game.send(text).await.is_err() {
                    return;
                }
            }
        }
        from_game.close();
    }));
    Ok(Link {
        outgoing,
        incoming,
        endpoint,
    })
}

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::spawn_local;

    /// The browser's handle on a link.
    #[wasm_bindgen]
    pub struct PadLink {
        outgoing: async_channel::Sender<String>,
        _endpoint: iroh::Endpoint,
    }

    #[wasm_bindgen]
    impl PadLink {
        /// Queue one JSON message for the game.
        pub fn send(
            &self,
            line: String,
        ) -> bool {
            self.outgoing.try_send(line).is_ok()
        }
    }

    /// Connect to the game and call `on_message(line)` for each message and `on_close()` at the end.
    #[wasm_bindgen]
    pub async fn connect(
        id: String,
        on_message: js_sys::Function,
        on_close: js_sys::Function,
    ) -> Result<PadLink, JsValue> {
        let link = crate::connect(&id, true, |task| spawn_local(task))
            .await
            .map_err(|e| JsValue::from_str(&e))?;
        let incoming = link.incoming.clone();
        spawn_local(async move {
            while let Ok(line) = incoming.recv().await {
                let _ = on_message.call1(&JsValue::NULL, &JsValue::from_str(&line));
            }
            let _ = on_close.call0(&JsValue::NULL);
        });
        Ok(PadLink {
            outgoing: link.outgoing,
            _endpoint: link.endpoint,
        })
    }
}
