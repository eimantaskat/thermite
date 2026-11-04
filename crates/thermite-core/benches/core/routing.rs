use criterion::{Criterion, criterion_group};
use http::{Method, StatusCode};
use thermite_core::{Parameter, RouteDescription, Router, Schema};

use crate::support::{bench_handle, request, router, runtime};

// the table is scanned in order, so the route registered last is the slowest to find
const ROUTE_COUNTS: [usize; 3] = [10, 100, 1000];

#[derive(Clone, Copy)]
enum Shape {
    Static,
    OneParameter,
    ThreeParameters,
}

impl Shape {
    const ALL: [Self; 3] = [Self::Static, Self::OneParameter, Self::ThreeParameters];

    fn name(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::OneParameter => "one_parameter",
            Self::ThreeParameters => "three_parameters",
        }
    }

    fn description(self, index: usize) -> RouteDescription {
        match self {
            Self::Static => RouteDescription::new(Method::GET, format!("/route{index}")),
            Self::OneParameter => {
                RouteDescription::new(Method::GET, format!("/route{index}/{{item_id}}"))
                    .parameter(Parameter::path("item_id", Schema::integer()))
            }
            Self::ThreeParameters => RouteDescription::new(
                Method::GET,
                format!("/route{index}/{{user_id}}/items/{{item_id}}/tags/{{tag}}"),
            )
            .parameter(Parameter::path("user_id", Schema::integer()))
            .parameter(Parameter::path("item_id", Schema::integer()))
            .parameter(Parameter::path("tag", Schema::string())),
        }
    }

    fn path(self, index: usize) -> String {
        match self {
            Self::Static => format!("/route{index}"),
            Self::OneParameter => format!("/route{index}/42"),
            Self::ThreeParameters => format!("/route{index}/7/items/42/tags/hot"),
        }
    }

    fn router(self, count: usize) -> Router {
        router((0..count).map(|index| self.description(index)))
    }
}

fn routing(criterion: &mut Criterion) {
    let runtime = runtime();
    let mut group = criterion.benchmark_group("routing");
    for count in ROUTE_COUNTS {
        let last = count - 1;
        for shape in Shape::ALL {
            let router = shape.router(count);
            for (position, index) in [("first", 0), ("last", last)] {
                let path = shape.path(index);
                bench_handle(
                    &mut group,
                    &runtime,
                    &format!("{}/{position}/{count}", shape.name()),
                    &router,
                    StatusCode::OK,
                    || request(http::Request::get(&path), ""),
                );
            }
        }

        let router = Shape::Static.router(count);
        bench_handle(
            &mut group,
            &runtime,
            &format!("not_found/{count}"),
            &router,
            StatusCode::NOT_FOUND,
            || request(http::Request::get("/missing"), ""),
        );
        let path = Shape::Static.path(last);
        bench_handle(
            &mut group,
            &runtime,
            &format!("method_not_allowed/{count}"),
            &router,
            StatusCode::METHOD_NOT_ALLOWED,
            || request(http::Request::post(&path), ""),
        );
    }
    group.finish();
}

criterion_group!(benches, routing);
