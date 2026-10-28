use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use thermite_core::{Arguments, Request, Response, RouteDescription};

use crate::extract::FromRequest;
use crate::response::IntoResponse;

pub trait Dependency: Clone + Send + 'static {
    type Needs: FromRequest;
    type Rejection: IntoResponse;

    fn resolve(needs: Self::Needs) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Depends<T>(pub T);

impl<T: Dependency> FromRequest for Depends<T> {
    type Rejection = Response;

    fn describe(description: RouteDescription) -> RouteDescription {
        description.dependency(T::Needs::describe)
    }

    async fn from_request(request: &Request, arguments: &Arguments) -> Result<Self, Response> {
        let slot = request
            .extensions()
            .get::<DependencyCache>()
            .and_then(DependencyCache::slot::<T>);
        // an extractor needing the same dependency meanwhile waits here, then takes the value
        let mut cached = match &slot {
            Some(slot) => Some(slot.lock().await),
            None => None,
        };
        if let Some(value) = cached.as_deref().and_then(Option::as_ref) {
            return Ok(Self(value.clone()));
        }
        let needs = T::Needs::from_request(request, arguments)
            .await
            .map_err(IntoResponse::into_response)?;
        let value = T::resolve(needs)
            .await
            .map_err(IntoResponse::into_response)?;
        if let Some(cached) = cached.as_mut() {
            **cached = Some(value.clone());
        }
        Ok(Self(value))
    }
}

type Slot<T> = Arc<tokio::sync::Mutex<Option<T>>>;

// a handler puts one in each request, so a dependency runs once however many extractors need it
#[derive(Clone, Default)]
pub(crate) struct DependencyCache(Arc<Mutex<HashMap<TypeId, Box<dyn Any + Send>>>>);

impl DependencyCache {
    // the map lock is released before anyone awaits a slot, so a std Mutex is enough for it
    fn slot<T: Send + 'static>(&self) -> Option<Slot<T>> {
        let mut slots = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        slots
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(Slot::<T>::default()))
            .downcast_ref::<Slot<T>>()
            .cloned()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::convert::Infallible;
    use std::future::ready;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use http::Method;
    use thermite_core::{Parameter, Schema};

    use super::*;

    struct Limit;

    impl FromRequest for Limit {
        type Rejection = Infallible;

        fn describe(description: RouteDescription) -> RouteDescription {
            description.parameter(Parameter::query("limit", Schema::integer()))
        }

        fn from_request(
            _: &Request,
            _: &Arguments,
        ) -> impl Future<Output = Result<Self, Infallible>> + Send {
            ready(Ok(Self))
        }
    }

    #[derive(Clone)]
    struct Paging;

    impl Dependency for Paging {
        type Needs = Limit;
        type Rejection = Infallible;

        fn resolve(_: Limit) -> impl Future<Output = Result<Self, Infallible>> + Send {
            ready(Ok(Self))
        }
    }

    #[derive(Clone)]
    struct Listing;

    impl Dependency for Listing {
        type Needs = (Depends<Paging>, Depends<Paging>);
        type Rejection = Infallible;

        fn resolve(_: Self::Needs) -> impl Future<Output = Result<Self, Infallible>> + Send {
            ready(Ok(Self))
        }
    }

    #[tokio::test]
    async fn declares_each_dependency_as_a_group_of_its_own() {
        let paging = |description: RouteDescription| {
            description
                .dependency(|paging| paging.parameter(Parameter::query("limit", Schema::integer())))
        };
        let description =
            Depends::<Listing>::describe(RouteDescription::new(Method::GET, "/items"));
        let resolved =
            Depends::<Listing>::from_request(&Request::default(), &Arguments::default()).await;

        assert_eq!(
            description,
            RouteDescription::new(Method::GET, "/items")
                .dependency(|listing| paging(paging(listing)))
        );
        assert!(resolved.is_ok());
    }

    static UNCACHED_RUNS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Clone)]
    struct Uncached;

    impl Dependency for Uncached {
        type Needs = ();
        type Rejection = Infallible;

        fn resolve((): ()) -> impl Future<Output = Result<Self, Infallible>> + Send {
            UNCACHED_RUNS.fetch_add(1, Ordering::SeqCst);
            ready(Ok(Self))
        }
    }

    #[tokio::test]
    async fn resolves_every_time_outside_a_handler() {
        let request = Request::default();
        let arguments = Arguments::default();

        Depends::<Uncached>::from_request(&request, &arguments)
            .await
            .unwrap();
        Depends::<Uncached>::from_request(&request, &arguments)
            .await
            .unwrap();

        assert_eq!(UNCACHED_RUNS.load(Ordering::SeqCst), 2);
    }
}
