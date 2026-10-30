// a test binary of its own: while one subscriber is registered, tracing caches whether a log call
// is wanted from whichever thread reaches it first, which in a shared binary is another test's

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use bytes::Bytes;
use http::StatusCode;
use thermite_core::{Request, Response, Server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

type Fields = BTreeMap<&'static str, String>;

// records the fields of every access event; tracing-subscriber would do this, but is a dependency
#[derive(Clone, Default)]
struct AccessEvents(Arc<Mutex<Vec<Fields>>>);

impl AccessEvents {
    fn recorded(&self) -> Vec<Fields> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct FieldText<'fields>(&'fields mut Fields);

impl Visit for FieldText<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.insert(field.name(), format!("{value:?}"));
    }
}

impl Subscriber for AccessEvents {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "thermite::access"
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields::new();
        fields.insert("level", event.metadata().level().to_string());
        event.record(&mut FieldText(&mut fields));
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(fields);
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

async fn send_raw(address: SocketAddr, request: &[u8]) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(address).await?;
    stream.write_all(request).await?;
    stream.shutdown().await?;
    stream.read_to_end(&mut Vec::new()).await?;
    Ok(())
}

// the subscriber is the test thread's default, and a current-thread runtime runs the server there too
#[tokio::test(flavor = "current_thread")]
async fn logs_one_event_per_request_with_what_uvicorn_shows() -> Result<(), Box<dyn Error>> {
    let events = AccessEvents::default();
    let _guard = tracing::subscriber::set_default(events.clone());
    let server = Server::bind("127.0.0.1:0").await?;
    let address = server.local_address()?;
    let handler = |request: Request| async move {
        assert_ne!(request.uri().path(), "/panic", "handler failed");
        Response::new(Bytes::new())
    };
    let task = tokio::spawn(server.serve(handler));
    let url = |path: &str| format!("http://{address}{path}");

    reqwest::get(url("/items?limit=5")).await?;
    let failed = reqwest::get(url("/panic")).await?;
    send_raw(
        address,
        b"POST /upload HTTP/1.0\r\ncontent-length: 10\r\n\r\nabc",
    )
    .await?;
    task.abort();

    let recorded = events.recorded();
    let shown: Vec<_> = recorded
        .iter()
        .map(|fields| {
            ["level", "message", "method", "path", "version", "status"]
                .map(|name| fields.get(name).map_or("", String::as_str))
        })
        .collect();
    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        shown,
        [
            [
                "INFO",
                "request",
                "GET",
                "/items?limit=5",
                "HTTP/1.1",
                "200"
            ],
            ["INFO", "request", "GET", "/panic", "HTTP/1.1", "500"],
            ["INFO", "request", "POST", "/upload", "HTTP/1.0", "400"],
        ]
    );
    for fields in &recorded {
        assert!(fields["client"].starts_with("127.0.0.1:"), "{fields:?}");
        assert!(fields["duration_ms"].parse::<f64>()? >= 0.0, "{fields:?}");
    }
    Ok(())
}
