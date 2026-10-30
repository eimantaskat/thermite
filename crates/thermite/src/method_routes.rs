// one method per HTTP method, each forwarding to `route` and returning what it returns
macro_rules! method_routes {
    (@each [$($attributes:tt)*] $output:ty;) => {};
    (@each [$($attributes:tt)*] $output:ty; $name:ident $method:ident $(, $rest:ident $rest_method:ident)*) => {
        $($attributes)*
        pub fn $name<Function, Extractors>(
            self,
            path: impl Into<String>,
            handler: Function,
        ) -> $output
        where
            Function: $crate::Handler<Extractors>,
            Extractors: 'static,
        {
            self.route($crate::Method::$method, path, handler)
        }

        $crate::method_routes::method_routes!(@each [$($attributes)*] $output; $($rest $rest_method),*);
    };
    ($(#[$attribute:meta])* $output:ty) => {
        $crate::method_routes::method_routes!(@each [$(#[$attribute])*] $output; get GET, post POST,
            put PUT, patch PATCH, delete DELETE, head HEAD, options OPTIONS, trace TRACE);
    };
}

pub(crate) use method_routes;
