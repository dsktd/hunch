//! Phone transport: an HTTPS + WebSocket server on the LAN, an iroh endpoint for phones on any
//! network (through n0's relays, page on GitHub Pages), and the join link and PIN for the QR code.
//!
//! Adapted from Gobble Grand Prix's `src/phone.rs` (<https://github.com/dsktd/gobble-grand-prix>,
//! Unlicense), without the engine: the server runs on its own thread with a small tokio runtime
//! and the game polls [`Server::poll`] and calls [`Server::send`].

use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::mpsc;

use axum::Router;
use axum::extract::Query as UrlQuery;
use axum::extract::State as AxumState;
use axum::extract::ws::Message as WsMessage;
use axum::extract::ws::WebSocket;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::StatusCode;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::routing::get;
use axum_server::tls_rustls::RustlsConfig;
use futures_util::SinkExt;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::protocol::VERSION;
use crate::protocol::Wire;

pub const PORT: u16 = 7777;
/// Ports tried after `PORT` when it is taken, before asking the OS for any free one.
const FALLBACKS: u16 = 10;
/// iroh application protocol for phones; carries the protocol version.
pub const ALPN: &[u8] = b"hunch/pad/1";
/// The phone page on GitHub Pages; the game id and PIN ride in the fragment so no server logs them.
pub const PAGES: &str = "https://dsktd.github.io/hunch/pad/";
const PAGE: &str = include_str!("../assets/phone/index.html");
/// Files the page loads next to itself.
const FILES: &[(&str, &str, &[u8])] = &[
    (
        "/fonts/LilitaOne-Regular.ttf",
        "font/ttf",
        include_bytes!("../assets/fonts/LilitaOne-Regular.ttf"),
    ),
    (
        "/fonts/AtkinsonHyperlegible-Regular.ttf",
        "font/ttf",
        include_bytes!("../assets/fonts/AtkinsonHyperlegible-Regular.ttf"),
    ),
    (
        "/fonts/AtkinsonHyperlegible-Bold.ttf",
        "font/ttf",
        include_bytes!("../assets/fonts/AtkinsonHyperlegible-Bold.ttf"),
    ),
];

/// What a phone did.
#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    Joined { id: u32, name: String },
    Left(u32),
    Wire(u32, Wire),
}

/// How phones join; the QR shows `anywhere` once iroh is up, `url` until then.
#[derive(Clone, Debug)]
pub struct JoinInfo {
    /// Same Wi-Fi only.
    pub url: String,
    /// Through iroh relays, from any network.
    pub anywhere: Option<String>,
    pub pin: String,
    pub lan: SocketAddr,
}
impl JoinInfo {
    pub fn qr_link(&self) -> &str {
        self.anywhere.as_deref().unwrap_or(&self.url)
    }
}

pub struct Server {
    pub join: JoinInfo,
    inbound: mpsc::Receiver<Inbound>,
    out: broadcast::Sender<String>,
    anywhere: mpsc::Receiver<String>,
    stop: Option<(
        tokio::sync::oneshot::Sender<()>,
        std::thread::JoinHandle<()>,
    )>,
}
impl Server {
    /// Start serving phones, or None without a LAN address or certificate.
    pub fn start() -> Option<Self> {
        let Some(ip) = lan_ip() else {
            eprintln!("hunch: no LAN address, phones off");
            return None;
        };
        let (cert, key) = certificate(ip)
            .map_err(|err| eprintln!("hunch: phone certificate: {err}"))
            .ok()?;
        let listener = bind_lan(ip)
            .map_err(|err| eprintln!("hunch: no LAN port on {ip}: {err}"))
            .ok();
        let port = listener
            .as_ref()
            .and_then(|l| l.local_addr().ok())
            .map_or(PORT, |a| a.port());
        let pin = format!("{:04}", rand::random_range(0..10_000));
        let addr = SocketAddr::new(ip, port);
        let (tx, inbound) = mpsc::channel();
        let (out, _) = broadcast::channel(512);
        let shared = Shared {
            pin: pin.clone(),
            tx,
            out: out.clone(),
            next: std::sync::Arc::default(),
            ids: std::sync::Arc::default(),
        };
        let (id_tx, anywhere) = mpsc::channel();
        let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(async move {
                // iroh lives until the game exits, whatever happens to the LAN server
                let endpoint = match iroh::Endpoint::builder(iroh::endpoint::presets::N0)
                    .relay_mode(relays())
                    .alpns(vec![ALPN.to_vec()])
                    .bind()
                    .await
                {
                    Ok(endpoint) => {
                        let _ = id_tx.send(endpoint.id().to_string());
                        tokio::spawn(iroh_accept(endpoint.clone(), shared.clone()));
                        Some(endpoint)
                    }
                    Err(err) => {
                        eprintln!("hunch: no iroh, phones need the LAN: {err}");
                        None
                    }
                };
                tokio::select! {
                    () = lan_server(listener, cert, key, addr, shared) => {
                        let _ = (&mut stopped).await;
                    }
                    _ = &mut stopped => {}
                }
                if let Some(endpoint) = endpoint {
                    endpoint.close().await;
                }
            });
        });
        Some(Self {
            join: JoinInfo {
                url: join_url(ip, port, &pin),
                anywhere: None,
                pin,
                lan: addr,
            },
            inbound,
            out,
            anywhere,
            stop: Some((stop, thread)),
        })
    }

    /// Everything phones did since the last call. Also picks up the anywhere link once iroh is up.
    pub fn poll(&mut self) -> Vec<Inbound> {
        if let Ok(id) = self.anywhere.try_recv() {
            self.join.anywhere = Some(anywhere_link(&id, &self.join.pin, self.join.lan));
        }
        self.inbound.try_iter().collect()
    }

    /// A message for the phones; one carrying `"id":N` reaches only phone N.
    pub fn send(
        &self,
        message: String,
    ) {
        let _ = self.out.send(message);
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some((stop, thread)) = self.stop.take() {
            let _ = stop.send(());
            let _ = thread.join();
        }
    }
}

fn anywhere_link(
    id: &str,
    pin: &str,
    lan: SocketAddr,
) -> String {
    format!(
        "{PAGES}?v={}#g={id}&pin={pin}&lan={lan}",
        env!("CARGO_PKG_VERSION")
    )
}

/// Dark modules of the QR code for `text`, row by row, with the side length.
pub fn qr(text: &str) -> Option<(usize, Vec<bool>)> {
    let code = qrcode::QrCode::new(text).ok()?;
    let side = code.width();
    let dark = code
        .to_colors()
        .iter()
        .map(|c| *c == qrcode::Color::Dark)
        .collect();
    Some((side, dark))
}

#[derive(Clone)]
struct Shared {
    pin: String,
    tx: mpsc::Sender<Inbound>,
    out: broadcast::Sender<String>,
    next: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// Phone token to connection id, so a reload gets its old seat back.
    ids: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, u32>>>,
}

/// The address phones reach us on: the interface of the default route. Nothing is sent.
fn lan_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip)
    }
}

/// A self-signed certificate for this IP, made once and kept beside the save data.
fn certificate(ip: IpAddr) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("hunch");
    let cert_path = dir.join(format!("cert-{ip}.pem"));
    let key_path = dir.join(format!("key-{ip}.pem"));
    if let (Ok(cert), Ok(key)) = (std::fs::read(&cert_path), std::fs::read(&key_path)) {
        return Ok((cert, key));
    }
    std::fs::create_dir_all(&dir)?;
    let made = rcgen::generate_simple_self_signed(vec![ip.to_string(), "hunch.local".to_string()])?;
    let cert = made.cert.pem().into_bytes();
    let key = made.signing_key.serialize_pem().into_bytes();
    std::fs::write(&cert_path, &cert)?;
    std::fs::write(&key_path, &key)?;
    Ok((cert, key))
}

fn join_url(
    ip: IpAddr,
    port: u16,
    pin: &str,
) -> String {
    format!("https://{ip}:{port}/?pin={pin}")
}

/// Claims the LAN port: `PORT`, the next few, then any free one. Another running instance
/// keeps its own port and its phones.
fn bind_lan(ip: IpAddr) -> std::io::Result<std::net::TcpListener> {
    let ports = (PORT..=PORT + FALLBACKS).chain([0]);
    let mut last = None;
    for port in ports {
        match std::net::TcpListener::bind(SocketAddr::new(ip, port)) {
            Ok(listener) => {
                listener.set_nonblocking(true)?;
                if port != PORT {
                    eprintln!(
                        "hunch: port {PORT} is taken (another instance running?), using {port}"
                    );
                }
                return Ok(listener);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => last = Some(err),
            Err(err) => return Err(err),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no port")))
}

/// The HTTPS page and websocket; returns when it fails or has no port.
async fn lan_server(
    listener: Option<std::net::TcpListener>,
    cert: Vec<u8>,
    key: Vec<u8>,
    addr: SocketAddr,
    shared: Shared,
) {
    let Some(listener) = listener else { return };
    let config = match RustlsConfig::from_pem(cert, key).await {
        Ok(config) => config,
        Err(err) => {
            eprintln!("hunch: phone TLS: {err}");
            return;
        }
    };
    let mut app = Router::new().route("/", get(page));
    for &(path, kind, bytes) in FILES {
        app = app.route(
            path,
            get(move || async move { ([(axum::http::header::CONTENT_TYPE, kind)], bytes) }),
        );
    }
    let app = app.route("/ws", get(socket)).with_state(shared);
    eprintln!("hunch: phones on https://{addr}");
    match axum_server::from_tcp_rustls(listener, config) {
        Ok(server) => {
            if let Err(err) = server.serve(app.into_make_service()).await {
                eprintln!("hunch: phone server stopped: {err}");
            }
        }
        Err(err) => eprintln!("hunch: phone server stopped: {err}"),
    }
}

async fn page() -> Html<&'static str> {
    Html(PAGE)
}

/// Join parameters: the LAN query string, or the first line on an iroh stream.
#[derive(Deserialize)]
struct Join {
    pin: String,
    /// Random id the page keeps in localStorage.
    #[serde(default)]
    token: String,
    #[serde(default)]
    name: String,
}

async fn socket(
    upgrade: WebSocketUpgrade,
    UrlQuery(join): UrlQuery<Join>,
    AxumState(shared): AxumState<Shared>,
) -> impl IntoResponse {
    if join.pin != shared.pin {
        return (StatusCode::FORBIDDEN, "Wrong PIN").into_response();
    }
    let id = claim(&shared, &join.token);
    upgrade.on_upgrade(move |ws| session(ws, shared, id, join.name))
}

fn hello(id: u32) -> String {
    format!(r#"{{"t":"hello","id":{id},"v":{VERSION}}}"#)
}

/// A pong for a ping, answered right here so latency never waits on the game.
fn pong(text: &str) -> Option<String> {
    match serde_json::from_str::<Wire>(text) {
        Ok(Wire::Ping { ms }) => Some(format!(r#"{{"t":"pong","ms":{ms}}}"#)),
        _ => None,
    }
}

async fn session(
    ws: WebSocket,
    shared: Shared,
    id: u32,
    name: String,
) {
    let _ = shared.tx.send(Inbound::Joined { id, name });
    let (mut sink, mut stream) = ws.split();
    let mut out = shared.out.subscribe();
    let _ = sink.send(WsMessage::Text(hello(id).into())).await;
    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(Ok(message)) = incoming else {
                    break;
                };
                let WsMessage::Text(text) = message else {
                    continue;
                };
                if let Some(reply) = pong(&text) {
                    if sink.send(WsMessage::Text(reply.into())).await.is_err() {
                        break;
                    }
                    continue;
                }
                let Ok(wire) = serde_json::from_str::<Wire>(&text) else {
                    continue;
                };
                if shared.tx.send(Inbound::Wire(id, wire)).is_err() {
                    break;
                }
            }
            outgoing = out.recv() => {
                let Ok(text) = outgoing else {
                    continue;
                };
                if for_phone(&text, id) && sink.send(WsMessage::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = shared.tx.send(Inbound::Left(id));
}

/// Messages carry an id when meant for one phone only.
fn for_phone(
    text: &str,
    id: u32,
) -> bool {
    !text.contains(r#""id":"#)
        || text.contains(&format!(r#""id":{id},"#))
        || text.contains(&format!(r#""id":{id}}}"#))
}

/// The connection id for a phone token, the same one again after a reconnect.
fn claim(
    shared: &Shared,
    token: &str,
) -> u32 {
    let mut ids = shared.ids.lock().expect("ids lock");
    let fresh = || {
        shared
            .next
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    };
    if token.is_empty() {
        fresh()
    } else {
        *ids.entry(token.chars().take(64).collect())
            .or_insert_with(fresh)
    }
}

/// n0's relays without the trailing dot iroh writes in their hostnames.
///
/// `WebKit` (every iOS browser) rejects the TLS certificate of `use1-1.relay.n0.iroh.link.`, and phones dial the
/// home relay this endpoint publishes. Same map as the pad's (`pad/src/lib.rs`).
pub fn relays() -> iroh::RelayMode {
    use iroh::defaults::prod;
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
    iroh::RelayMode::Custom(iroh::RelayMap::from_iter(urls))
}

/// Accept phones over iroh: relayed or direct QUIC, newline-delimited JSON on one stream.
async fn iroh_accept(
    endpoint: iroh::Endpoint,
    shared: Shared,
) {
    while let Some(incoming) = endpoint.accept().await {
        let shared = shared.clone();
        tokio::spawn(async move {
            let Ok(connection) = incoming.await else {
                return;
            };
            let Ok((send, recv)) = connection.accept_bi().await else {
                return;
            };
            iroh_session(send, recv, shared).await;
            drop(connection);
        });
    }
}

async fn iroh_session(
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    shared: Shared,
) {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 4096];
    let mut id = None;
    let mut out = shared.out.subscribe();
    loop {
        tokio::select! {
            read = recv.read(&mut chunk) => {
                let Ok(Some(n)) = read else {
                    break;
                };
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 65_536 {
                    break;
                }
                while let Some(end) = buf.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=end).collect();
                    let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                    match id {
                        // The first line must carry the PIN
                        None => {
                            let Ok(join) = serde_json::from_str::<Join>(&text) else {
                                return;
                            };
                            if join.pin != shared.pin {
                                let _ = send.write_all(b"{\"t\":\"denied\"}\n").await;
                                let _ = send.finish();
                                return;
                            }
                            let phone = claim(&shared, &join.token);
                            id = Some(phone);
                            let _ = shared.tx.send(Inbound::Joined { id: phone, name: join.name });
                            let _ = send.write_all(format!("{}\n", hello(phone)).as_bytes()).await;
                        }
                        Some(phone) => {
                            if let Some(reply) = pong(&text) {
                                if send.write_all(format!("{reply}\n").as_bytes()).await.is_err() {
                                    return;
                                }
                                continue;
                            }
                            let Ok(wire) = serde_json::from_str::<Wire>(&text) else {
                                continue;
                            };
                            if shared.tx.send(Inbound::Wire(phone, wire)).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            outgoing = out.recv(), if id.is_some() => {
                let Ok(text) = outgoing else {
                    continue;
                };
                let phone = id.unwrap_or_default();
                if for_phone(&text, phone) && send.write_all(format!("{text}\n").as_bytes()).await.is_err() {
                    break;
                }
            }
        }
    }
    if let Some(phone) = id {
        let _ = shared.tx.send(Inbound::Left(phone));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_when_the_port_is_taken() {
        let ip: IpAddr = [127, 0, 0, 1].into();
        // Hold the default port ourselves (or find it already held).
        let _held = std::net::TcpListener::bind((ip, PORT));
        let got = bind_lan(ip).unwrap();
        let port = got.local_addr().unwrap().port();
        assert_ne!(port, PORT);
        assert!(join_url(ip, port, "1234").contains(&format!(":{port}/")));
    }

    #[test]
    fn lan_failure_leaves_the_runtime_alive() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (tx, _rx) = mpsc::channel();
        let shared = Shared {
            pin: String::new(),
            tx,
            out: broadcast::channel(1).0,
            next: std::sync::Arc::default(),
            ids: std::sync::Arc::default(),
        };
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        runtime.block_on(lan_server(None, vec![], vec![], addr, shared));
    }

    #[test]
    fn messages_reach_their_phone_and_pings_get_pongs() {
        assert!(for_phone(r#"{"t":"view","id":3,"seq":1}"#, 3));
        assert!(!for_phone(r#"{"t":"view","id":31,"seq":1}"#, 3));
        assert!(for_phone(r#"{"t":"full","id":3}"#, 3));
        assert!(for_phone(r#"{"t":"tick"}"#, 3));
        assert_eq!(
            pong(r#"{"t":"ping","ms":5}"#).as_deref(),
            Some(r#"{"t":"pong","ms":5}"#)
        );
        assert_eq!(pong(r#"{"t":"start"}"#), None);
        assert!(hello(2).contains(&format!(r#""v":{VERSION}"#)));
    }

    #[test]
    fn reconnects_get_the_same_id_and_links_hide_the_pin_in_the_fragment() {
        let (tx, _rx) = mpsc::channel();
        let shared = Shared {
            pin: "1".into(),
            tx,
            out: broadcast::channel(1).0,
            next: std::sync::Arc::default(),
            ids: std::sync::Arc::default(),
        };
        let a = claim(&shared, "tok");
        assert_eq!(claim(&shared, "tok"), a);
        assert_ne!(claim(&shared, "other"), a);
        assert_ne!(claim(&shared, ""), claim(&shared, ""));
        let link = anywhere_link("abc", "0420", SocketAddr::from(([10, 0, 0, 2], 7777)));
        assert!(link.starts_with(PAGES));
        assert!(link.split('#').nth(1).unwrap().contains("pin=0420"));
        let (side, dark) = qr(&link).unwrap();
        assert_eq!(dark.len(), side * side);
    }
}
