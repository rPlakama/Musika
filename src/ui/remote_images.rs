//! GPUI image HTTP client.
//!
//! GPUI's default application has a `NullHttpClient`, so an `img(http://…)`
//! would never load. Cover payloads deliberately fall back to the provider's
//! `getCoverArt` URL while the local cover cache is cold; this small adapter
//! lets GPUI stream those images through the same reqwest stack as the rest of
//! the app.
//!
//! GPUI polls its image futures on its own worker pool, where no Tokio reactor
//! exists. Every reqwest call is therefore moved onto the application runtime
//! and only the oneshot result is awaited from the GPUI future.

use std::io::Read;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use gpui::http_client::{AsyncBody, HttpClient, Inner, Request, Response, Url, http::HeaderValue};

pub struct ReqwestHttpClient {
    client: reqwest::Client,
    user_agent: HeaderValue,
}

impl ReqwestHttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
            user_agent: HeaderValue::from_static(concat!("Musika/", env!("CARGO_PKG_VERSION"))),
        }
    }
}

impl HttpClient for ReqwestHttpClient {
    fn type_name(&self) -> &'static str {
        "musika-reqwest"
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let client = self.client.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        crate::app::runtime().spawn(async move {
            _ = tx.send(fetch(client, request).await);
        });
        Box::pin(async move {
            rx.await
                .map_err(|_| anyhow::anyhow!("GPUI image HTTP task cancelled"))?
        })
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }
}

async fn fetch(
    client: reqwest::Client,
    request: Request<AsyncBody>,
) -> anyhow::Result<Response<AsyncBody>> {
    let (parts, body) = request.into_parts();
    let body_bytes = match body.0 {
        Inner::Empty => Vec::new(),
        Inner::Bytes(mut cursor) => {
            let mut bytes = Vec::new();
            cursor.read_to_end(&mut bytes)?;
            bytes
        }
        Inner::AsyncReader(_) => {
            anyhow::bail!("streaming image request bodies are not supported")
        }
    };
    let mut builder = client
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers);
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes);
    }
    let response = builder.send().await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.bytes().await?;
    let mut out = Response::new(AsyncBody::from_bytes(bytes));
    *out.status_mut() = status;
    *out.headers_mut() = headers;
    Ok(out)
}

pub fn app_http_client() -> Arc<dyn HttpClient> {
    Arc::new(ReqwestHttpClient::new())
}
