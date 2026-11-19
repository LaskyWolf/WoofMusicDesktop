//! Listen Together (`Trop/ListenTogether/`): the metroserver protocol and
//! the WebSocket it travels on. The room itself lives in the app, next to
//! the player it drives.

pub mod protocol;

use std::time::Duration;

use futures_util::{SinkExt, Stream, StreamExt, stream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Message, http::header::USER_AGENT};

/// Metrolist's public server, so rooms can mix both apps.
pub const DEFAULT_SERVER_URL: &str = "wss://metroserverx.meowery.eu/ws";

#[derive(Debug, Clone)]
pub enum Event {
    /// A message type and its payload.
    Message(String, Vec<u8>),
    /// The socket is gone: closed by either side or failed. Always last.
    Closed(String),
}

/// Sends envelopes on one connection. Dropping it closes the socket.
#[derive(Debug, Clone)]
pub struct Outbox(mpsc::UnboundedSender<Vec<u8>>);

impl Outbox {
    pub fn send(&self, kind: &str, payload: &[u8]) {
        let _ = self.0.send(protocol::Envelope::encode(kind, payload));
    }
}

/// `ListenTogetherConnection`: one WebSocket to a metroserver. What is sent
/// before the socket opens waits for it. The stream connects when first
/// polled and ends after `Event::Closed`.
pub fn connect(url: String, user_agent: String) -> (Outbox, impl Stream<Item = Event> + Send) {
    let (outgoing, outbox) = mpsc::unbounded_channel();
    let (events, incoming) = mpsc::unbounded_channel();
    let run = stream::once(async move {
        let reason = session(&url, &user_agent, outbox, &events)
            .await
            .unwrap_or_else(|error| error);
        let _ = events.send(Event::Closed(reason));
    })
    .filter_map(|()| async { None });
    let incoming = stream::unfold(incoming, |mut rx| async {
        rx.recv().await.map(|e| (e, rx))
    });
    (Outbox(outgoing), stream::select(run, incoming))
}

async fn session(
    url: &str,
    user_agent: &str,
    mut outbox: mpsc::UnboundedReceiver<Vec<u8>>,
    events: &mpsc::UnboundedSender<Event>,
) -> Result<String, String> {
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    // metroserver's operators can set rules per client; this names the app.
    if let Ok(agent) = HeaderValue::from_str(user_agent) {
        request.headers_mut().insert(USER_AGENT, agent);
    }
    let (socket, _) = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| "the server did not answer".to_owned())?
    .map_err(|e| e.to_string())?;
    let (mut sink, mut source) = socket.split();
    loop {
        tokio::select! {
            out = outbox.recv() => match out {
                Some(bytes) => sink
                    .send(Message::Binary(bytes.into()))
                    .await
                    .map_err(|e| format!("send failed: {e}"))?,
                None => {
                    let _ = sink.send(Message::Close(None)).await;
                    return Ok("closed by the app".to_owned());
                }
            },
            message = source.next() => {
                let data = match message {
                    Some(Ok(Message::Binary(data))) => data.to_vec(),
                    Some(Ok(Message::Text(text))) => text.as_bytes().to_vec(),
                    Some(Ok(Message::Close(frame))) => {
                        let code = frame.map_or(1005, |f| u16::from(f.code));
                        return Ok(format!("server closed the connection ({code})"));
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => return Err(error.to_string()),
                    None => return Ok("server closed the connection".to_owned()),
                };
                if let Ok(envelope) = protocol::Envelope::decode(&data)
                    && !envelope.compressed
                {
                    let _ = events.send(Event::Message(envelope.kind, envelope.payload));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// A pretend metroserver on localhost answers the capabilities, and the
    /// client hears it and then the close.
    #[tokio::test]
    async fn a_connection_carries_envelopes_both_ways() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let Some(Ok(Message::Binary(data))) = ws.next().await else {
                panic!("no capabilities");
            };
            let hello = protocol::Envelope::decode(&data).unwrap();
            assert_eq!(hello.kind, protocol::kind::CLIENT_CAPABILITIES);
            let reply = protocol::Envelope::encode(protocol::kind::SERVER_CAPABILITIES, &[]);
            ws.send(Message::Binary(reply.into())).await.unwrap();
            ws.close(None).await.unwrap();
        });
        let (outbox, events) = connect(format!("ws://{address}"), "WoofMusic test".into());
        outbox.send(
            protocol::kind::CLIENT_CAPABILITIES,
            &protocol::client_capabilities("WoofMusic 1", &[]),
        );
        let events: Vec<Event> = events.collect().await;
        server.await.unwrap();
        assert!(
            matches!(&events[0], Event::Message(kind, _) if kind == protocol::kind::SERVER_CAPABILITIES)
        );
        assert!(matches!(events.last(), Some(Event::Closed(_))));
    }

    /// Garbage, a compressed envelope and a 5 MB frame are skipped; the
    /// valid message after them still arrives, and dropping the outbox ends
    /// the stream (no task is left running).
    #[tokio::test]
    async fn bad_frames_are_skipped_and_dropping_the_outbox_ends_the_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let mut frames = vec![
                Message::Binary(vec![0xFF; 20].into()),
                Message::Text("{\"not\":\"protobuf\"}".into()),
                Message::Binary(vec![7u8; 5_000_000].into()),
                Message::Binary(protocol::Envelope::encode("x", &[0x0A, 0xFF, 0xFF]).into()),
            ];
            let mut compressed = protocol::Writer::default();
            compressed.string(1, "zipped");
            compressed.bool(3, true);
            frames.push(Message::Binary(compressed.finish().into()));
            frames.push(Message::Binary(
                protocol::Envelope::encode(protocol::kind::PONG, &[]).into(),
            ));
            for frame in frames {
                ws.send(frame).await.unwrap();
            }
            // Waits for the client to hang up.
            while let Some(Ok(message)) = ws.next().await {
                if message.is_close() {
                    break;
                }
            }
        });
        let (outbox, events) = connect(format!("ws://{address}"), "WoofMusic test".into());
        let mut events = Box::pin(events);
        let mut kinds = Vec::new();
        while let Some(Event::Message(kind, _)) = events.next().await {
            kinds.push(kind.clone());
            if kind == protocol::kind::PONG {
                break;
            }
        }
        assert_eq!(kinds, ["x", protocol::kind::PONG]);
        drop(outbox);
        let rest: Vec<Event> = tokio::time::timeout(Duration::from_secs(5), events.collect())
            .await
            .expect("the stream must end");
        assert!(matches!(rest.as_slice(), [Event::Closed(_)]));
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("the server saw the close")
            .unwrap();
    }

    /// Nothing listens there: one `Closed`, then the stream ends (no retry loop).
    #[tokio::test]
    async fn a_refused_connection_closes_once() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let (_outbox, events) = connect(format!("ws://{address}"), "WoofMusic test".into());
        let events: Vec<Event> = tokio::time::timeout(Duration::from_secs(5), events.collect())
            .await
            .expect("must not hang");
        assert!(matches!(events.as_slice(), [Event::Closed(_)]));
    }
}
