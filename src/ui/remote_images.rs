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
//!
//! Requests opt out of content-encoding negotiation (`Accept-Encoding:
//! identity`): these are image bytes, already compressed, and the JSON API
//! gzip path is owned by [`crate::http_client`].

use std::io::Read;
use std::sync::Arc;

use futures_util::future::{AbortHandle, Abortable, BoxFuture};
use gpui::http_client::{
    AsyncBody, HttpClient, Inner, Request, Response, Url, http::HeaderValue,
    http::header::ACCEPT_ENCODING,
};

/// Aborts the background HTTP task when the GPUI image future is dropped.
///
/// GPUI may abandon an image load at any time (a page navigated away, a cover
/// reloaded for a bigger edge, an overlay closed). Without this guard the
/// reqwest task for the abandoned URL would keep running to completion because
/// the oneshot sender stays owned by the spawned task.
struct CancelOnDrop {
    handle: Option<AbortHandle>,
}

impl CancelOnDrop {
    fn new(handle: AbortHandle) -> Self {
        Self {
            handle: Some(handle),
        }
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

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
        mut request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        // Cover streams are already-compressed JPEG/PNG bytes; the crate-wide
        // `gzip` feature would otherwise advertise gzip for them. Ask for the
        // raw bytes so the image decoder never sees an inflate step first.
        request
            .headers_mut()
            .insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let client = self.client.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        crate::app::runtime().spawn(async move {
            let result = Abortable::new(fetch(client, request), abort_registration).await;
            let result = match result {
                Ok(result) => result,
                Err(_aborted) => Err(anyhow::anyhow!("GPUI image HTTP request aborted")),
            };
            _ = tx.send(result);
        });
        // Held inside the returned future: dropping the future (GPUI abandoning
        // the image) drops the guard and cancels the in-flight request.
        let cancel = CancelOnDrop::new(abort_handle);
        Box::pin(async move {
            let _cancel = cancel;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future as _;
    use std::task::{Context, Poll};

    #[test]
    fn dropping_an_abandoned_request_aborts_the_http_task() {
        let (abort_handle, registration) = AbortHandle::new_pair();
        let cancel = CancelOnDrop::new(abort_handle);
        let mut request = Box::pin(Abortable::new(std::future::pending::<()>(), registration));
        let waker = futures_util::task::noop_waker();
        let mut context = Context::from_waker(&waker);
        assert!(
            request.as_mut().poll(&mut context).is_pending(),
            "a pending HTTP request stays pending while the image future is alive"
        );

        // GPUI abandoned the image: the returned future (and its guard) is
        // dropped, which must cancel the request instead of letting it run.
        drop(cancel);
        assert!(
            matches!(request.as_mut().poll(&mut context), Poll::Ready(Err(_))),
            "the aborted HTTP request resolves as cancelled"
        );
    }
}
