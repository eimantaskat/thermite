use std::error::Error;
use std::future::{Future, ready};

use crate::state::FromState;

pub trait Resource: Send + Sync + Sized + 'static {
    type Needs: FromState;
    type Error: Error + Send + Sync + 'static;

    fn start(needs: Self::Needs) -> impl Future<Output = Result<Self, Self::Error>> + Send;

    fn stop(&self) -> impl Future<Output = ()> + Send {
        ready(())
    }
}
