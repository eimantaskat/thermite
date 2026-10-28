// calls `$target!` once per prefix of the list: no type parameters, then one, up to all sixteen
macro_rules! for_each_arity {
    ($target:ident) => {
        $crate::arity::for_each_arity!($target; [] Extractor1 Extractor2 Extractor3 Extractor4
            Extractor5 Extractor6 Extractor7 Extractor8 Extractor9 Extractor10 Extractor11
            Extractor12 Extractor13 Extractor14 Extractor15 Extractor16);
    };
    ($target:ident; [$($done:ident),*]) => {
        $target!($($done),*);
    };
    ($target:ident; [$($done:ident),*] $next:ident $($rest:ident)*) => {
        $target!($($done),*);
        $crate::arity::for_each_arity!($target; [$($done,)* $next] $($rest)*);
    };
}

pub(crate) use for_each_arity;
