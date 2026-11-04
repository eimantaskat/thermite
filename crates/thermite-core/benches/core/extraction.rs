use criterion::{Criterion, criterion_group};
use http::header::COOKIE;
use http::{Method, StatusCode};
use thermite_core::{Parameter, RouteDescription, Schema};

use crate::support::{bench_handle, request, router, runtime};

const MANY: usize = 10;
const FEW: usize = 5;

fn names(prefix: &str, count: usize) -> impl Iterator<Item = String> {
    (0..count).map(move |index| format!("{prefix}{index}"))
}

fn route(path: &str, parameters: impl IntoIterator<Item = Parameter>) -> RouteDescription {
    parameters.into_iter().fold(
        RouteDescription::new(Method::GET, path),
        RouteDescription::parameter,
    )
}

fn extraction(criterion: &mut Criterion) {
    let runtime = runtime();
    let router = router([
        route("/none", []),
        route("/query/one", [Parameter::query("q", Schema::string())]),
        route(
            "/query/many",
            names("p", MANY).map(|name| Parameter::query(name, Schema::integer())),
        ),
        route(
            "/query/list",
            [Parameter::query("tag", Schema::array(Schema::string()))],
        ),
        route(
            "/headers",
            names("x-header-", FEW).map(|name| Parameter::header(name, Schema::string())),
        ),
        route(
            "/cookies",
            names("cookie", FEW).map(|name| Parameter::cookie(name, Schema::string())),
        ),
    ]);
    let many_query = format!(
        "/query/many?{}",
        names("p", MANY)
            .enumerate()
            .map(|(index, name)| format!("{name}={index}"))
            .collect::<Vec<_>>()
            .join("&")
    );
    let list_query = format!(
        "/query/list?{}",
        names("tag=t", MANY).collect::<Vec<_>>().join("&")
    );
    let cookies = names("cookie", FEW)
        .map(|name| format!("{name}=value"))
        .collect::<Vec<_>>()
        .join("; ");

    let mut group = criterion.benchmark_group("extraction");
    let mut bench = |name: &str, build: &dyn Fn() -> http::request::Builder| {
        bench_handle(&mut group, &runtime, name, &router, StatusCode::OK, || {
            request(build(), "")
        });
    };
    bench("none", &|| http::Request::get("/none"));
    bench("query/one", &|| http::Request::get("/query/one?q=hello"));
    bench("query/many", &|| http::Request::get(&many_query));
    bench("query/list", &|| http::Request::get(&list_query));
    bench("headers", &|| {
        names("x-header-", FEW).fold(http::Request::get("/headers"), |builder, name| {
            builder.header(name, "value")
        })
    });
    bench("cookies", &|| {
        http::Request::get("/cookies").header(COOKIE, &cookies)
    });
    group.finish();
}

criterion_group!(benches, extraction);
