use std::any::{Any, TypeId};
use std::fmt;
use std::sync::Arc;

// turns a JSON body straight into what the handler takes, skipping validation for bodies that need none
pub trait BodyDecoder: Any + Send + Sync {
    // None hands the body to validation, which coerces it or writes the 422; Some must hold only what validation accepts
    fn decode(&self, body: &[u8]) -> Option<Box<dyn Any + Send>>;
}

#[derive(Clone)]
pub(crate) struct Decoder(Arc<dyn BodyDecoder>);

impl Decoder {
    pub(crate) fn new(decoder: impl BodyDecoder) -> Self {
        Self(Arc::new(decoder))
    }

    pub(crate) fn decode(&self, body: &[u8]) -> Option<Box<dyn Any + Send>> {
        self.0.decode(body)
    }

    fn type_id(&self) -> TypeId {
        let decoder: &dyn Any = &*self.0;
        decoder.type_id()
    }
}

// decoders of one type decode alike, so describing a route twice gives equal descriptions
impl PartialEq for Decoder {
    fn eq(&self, other: &Self) -> bool {
        self.type_id() == other.type_id()
    }
}

impl Eq for Decoder {}

impl fmt::Debug for Decoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Decoder")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Integer;

    impl BodyDecoder for Integer {
        fn decode(&self, body: &[u8]) -> Option<Box<dyn Any + Send>> {
            let text = std::str::from_utf8(body).ok()?;
            Some(Box::new(text.parse::<i64>().ok()?))
        }
    }

    struct Text;

    impl BodyDecoder for Text {
        fn decode(&self, body: &[u8]) -> Option<Box<dyn Any + Send>> {
            Some(Box::new(String::from_utf8_lossy(body).into_owned()))
        }
    }

    #[test]
    fn decoders_are_equal_when_their_types_are() {
        assert_eq!(Decoder::new(Integer), Decoder::new(Integer));
        assert_ne!(Decoder::new(Integer), Decoder::new(Text));
        assert_eq!(format!("{:?}", Decoder::new(Text)), "Decoder");
    }

    #[test]
    fn hands_back_what_the_decoder_made() {
        let decoded = Decoder::new(Integer).decode(b"42");

        assert_eq!(
            decoded.and_then(|value| value.downcast::<i64>().ok()),
            Some(Box::new(42))
        );
        assert!(Decoder::new(Integer).decode(b"forty-two").is_none());
        assert!(Decoder::new(Text).decode(b"text").is_some());
    }
}
