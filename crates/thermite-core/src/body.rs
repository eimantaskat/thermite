use std::error::Error;
use std::fmt;
use std::mem;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use futures_util::stream::{Stream, StreamExt};

pub type BoxError = Box<dyn Error + Send + Sync>;

// the error a streamed body failed with, as a type of its own so that `?` converts it
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct BodyError(BoxError);

impl BodyError {
    // what the stream failed with, for a test to downcast
    #[must_use]
    pub fn get_ref(&self) -> &(dyn Error + Send + Sync + 'static) {
        self.0.as_ref()
    }
}

type ChunkStream = Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>;

// the body of a response: whole, or written chunk by chunk as a stream produces it
#[derive(Default)]
pub struct Body(Inner);

enum Inner {
    Full(Bytes),
    // the mutex makes a response Sync without asking it of the stream, and is never locked, as polling holds the body mutably
    Stream(Mutex<ChunkStream>),
}

impl Default for Inner {
    fn default() -> Self {
        Self::Full(Bytes::new())
    }
}

impl Body {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    // an error ends the body early, so the connection is closed, since the status was sent before it
    pub fn from_stream<Chunks, Chunk, StreamError>(chunks: Chunks) -> Self
    where
        Chunks: Stream<Item = Result<Chunk, StreamError>> + Send + 'static,
        Chunk: Into<Bytes>,
        StreamError: Into<BoxError>,
    {
        // fused, since many streams panic when polled again after they ended
        let chunks = chunks
            .map(|chunk| chunk.map(Into::into).map_err(Into::into))
            .fuse();
        Self(Inner::Stream(Mutex::new(Box::pin(chunks))))
    }

    // `None` for a body still to be streamed
    #[must_use]
    pub fn as_bytes(&self) -> Option<&Bytes> {
        match &self.0 {
            Inner::Full(bytes) => Some(bytes),
            Inner::Stream(_) => None,
        }
    }

    #[must_use]
    pub fn is_stream(&self) -> bool {
        matches!(self.0, Inner::Stream(_))
    }

    pub async fn chunk(&mut self) -> Option<Result<Bytes, BodyError>> {
        self.next().await.map(|chunk| chunk.map_err(BodyError))
    }

    // a stream of one chunk gives that chunk without copying it
    pub async fn into_bytes(mut self) -> Result<Bytes, BodyError> {
        let Some(first) = self.chunk().await.transpose()? else {
            return Ok(Bytes::new());
        };
        let Some(second) = self.chunk().await.transpose()? else {
            return Ok(first);
        };
        let mut body = BytesMut::from(first);
        body.extend_from_slice(&second);
        while let Some(chunk) = self.chunk().await {
            body.extend_from_slice(&chunk?);
        }
        Ok(body.freeze())
    }
}

// a whole body comes out as one chunk, an empty one as none
impl Stream for Body {
    type Item = Result<Bytes, BoxError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match &mut self.get_mut().0 {
            Inner::Full(bytes) if bytes.is_empty() => Poll::Ready(None),
            Inner::Full(bytes) => Poll::Ready(Some(Ok(mem::take(bytes)))),
            Inner::Stream(chunks) => chunks
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner)
                .as_mut()
                .poll_next(cx),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            Inner::Full(bytes) => {
                let chunks = usize::from(!bytes.is_empty());
                (chunks, Some(chunks))
            }
            Inner::Stream(_) => (0, None),
        }
    }
}

impl From<Bytes> for Body {
    fn from(bytes: Bytes) -> Self {
        Self(Inner::Full(bytes))
    }
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Self {
        Bytes::from(bytes).into()
    }
}

impl From<String> for Body {
    fn from(text: String) -> Self {
        Bytes::from(text).into()
    }
}

impl From<&'static str> for Body {
    fn from(text: &'static str) -> Self {
        Bytes::from_static(text.as_bytes()).into()
    }
}

impl From<&'static [u8]> for Body {
    fn from(bytes: &'static [u8]) -> Self {
        Bytes::from_static(bytes).into()
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Inner::Full(bytes) => formatter.debug_tuple("Body").field(bytes).finish(),
            Inner::Stream(_) => formatter.write_str("Body(<stream>)"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::convert::Infallible;
    use std::io;

    use futures_util::stream;

    use super::*;

    #[tokio::test]
    async fn a_whole_body_is_one_chunk_and_an_empty_one_none() {
        let mut whole = Body::from("hello");
        let mut empty = Body::empty();

        assert_eq!(whole.as_bytes(), Some(&Bytes::from_static(b"hello")));
        assert_eq!(whole.size_hint(), (1, Some(1)));
        assert_eq!(whole.next().await.unwrap().unwrap(), "hello");
        assert!(whole.next().await.is_none());
        assert_eq!(empty.size_hint(), (0, Some(0)));
        assert!(empty.next().await.is_none());
        assert!(!empty.is_stream());
    }

    #[tokio::test]
    async fn a_stream_is_read_chunk_by_chunk_or_whole() {
        let chunks = || stream::iter(["a", "b"].map(Ok::<_, Infallible>));
        let mut streamed = Body::from_stream(chunks());

        assert!(streamed.is_stream());
        assert_eq!(streamed.as_bytes(), None);
        assert_eq!(streamed.size_hint(), (0, None));
        assert_eq!(streamed.next().await.unwrap().unwrap(), "a");
        assert_eq!(
            Body::from_stream(chunks()).into_bytes().await.unwrap(),
            "ab"
        );
    }

    #[tokio::test]
    async fn an_error_ends_reading_a_stream() {
        let chunks = stream::iter([Ok("a"), Err(io::Error::other("lost"))]);

        let read = Body::from_stream(chunks).into_bytes().await;

        let error = read.unwrap_err();
        assert_eq!(error.to_string(), "lost");
        assert!(error.get_ref().downcast_ref::<io::Error>().is_some());
    }

    #[tokio::test]
    async fn reads_a_stream_of_any_length_whole() {
        let read = |chunks: &'static [&'static str]| {
            Body::from_stream(stream::iter(
                chunks.iter().copied().map(Ok::<_, Infallible>),
            ))
            .into_bytes()
        };

        assert_eq!(read(&[]).await.unwrap(), "");
        assert_eq!(read(&["one"]).await.unwrap(), "one");
        assert_eq!(read(&["a", "b", "c"]).await.unwrap(), "abc");
    }

    // unfold panics when polled again after it ended
    #[tokio::test]
    async fn a_stream_that_ended_stays_ended() {
        let chunks = stream::unfold(false, |done| async move {
            (!done).then_some((Ok::<_, Infallible>("once"), true))
        });
        let mut body = Body::from_stream(chunks);

        assert_eq!(body.chunk().await.unwrap().unwrap(), "once");
        assert!(body.chunk().await.is_none());
        assert!(body.chunk().await.is_none());
    }

    #[test]
    fn a_response_can_be_shared_between_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<crate::Response>();
    }

    #[test]
    fn converts_from_what_a_whole_body_is_made_of() {
        let bodies = [
            Body::from(Bytes::from_static(b"x")),
            Body::from(vec![b'x']),
            Body::from("x".to_owned()),
            Body::from(&b"x"[..]),
        ];

        for body in bodies {
            assert_eq!(body.as_bytes().unwrap(), "x");
        }
        assert_eq!(format!("{:?}", Body::from("x")), "Body(b\"x\")");
        let stream = Body::from_stream(stream::empty::<Result<Bytes, Infallible>>());
        assert_eq!(format!("{stream:?}"), "Body(<stream>)");
    }
}
