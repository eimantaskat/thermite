use std::borrow::Cow;
use std::future::{Future, ready};

use serde::de::DeserializeOwned;
use serde::de::value::MapDeserializer;
use thermite_core::{Arguments, Parameter, ParameterLocation, Request, RouteDescription};

use crate::extract::FromRequest;
use crate::model::Model;
use crate::rejection::ModelMismatch;

macro_rules! parameter_extractors {
    ($($extractor:ident),*) => {
        $(
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
            pub struct $extractor<T>(pub T);

            impl<T: Model + DeserializeOwned + Send> FromRequest for $extractor<T> {
                type Rejection = ModelMismatch;

                fn describe(description: RouteDescription) -> RouteDescription {
                    describe_fields::<T>(description, ParameterLocation::$extractor)
                }

                fn from_request(
                    _: &Request,
                    arguments: &Arguments,
                ) -> impl Future<Output = Result<Self, ModelMismatch>> + Send {
                    ready(read_fields(arguments, ParameterLocation::$extractor).map(Self))
                }
            }
        )*
    };
}

parameter_extractors!(Path, Query, Header, Cookie);

// like FastAPI's Header, the field `user_agent` reads the header `user-agent`
fn name_in_request(location: ParameterLocation, field_name: &str) -> Cow<'_, str> {
    match location {
        ParameterLocation::Header => Cow::Owned(field_name.replace('_', "-")),
        _ => Cow::Borrowed(field_name),
    }
}

fn describe_fields<T: Model>(
    description: RouteDescription,
    location: ParameterLocation,
) -> RouteDescription {
    T::fields()
        .into_iter()
        .fold(description, |description, field| {
            let name = name_in_request(location, field.name()).into_owned();
            description.parameter(Parameter::new(location, field.with_name(name)))
        })
}

fn read_fields<T: Model + DeserializeOwned>(
    arguments: &Arguments,
    location: ParameterLocation,
) -> Result<T, ModelMismatch> {
    let fields = T::fields();
    // pairs of field names and borrowed values deserialize like an object, without copying the values
    let values = fields.iter().filter_map(|field| {
        arguments
            .parameter(location, &name_in_request(location, field.name()))
            .map(|value| (field.name(), value))
    });
    T::deserialize(MapDeserializer::<_, serde_json::Error>::new(values))
        .map_err(ModelMismatch::new::<T>)
}

#[cfg(test)]
mod tests {
    use http::Method;
    use thermite_core::{Field, Schema};

    use super::*;
    use crate::model::ToSchema;

    struct Client;

    impl Model for Client {
        fn fields() -> Vec<Field> {
            vec![
                Field::new("user_agent", String::schema()),
                Field::new("x_tag", Vec::<String>::schema()).optional(),
            ]
        }
    }

    fn described(location: ParameterLocation) -> RouteDescription {
        let description = RouteDescription::new(Method::GET, "/items/{user_agent}");
        describe_fields::<Client>(description, location)
    }

    #[test]
    fn declares_each_field_in_its_location() {
        let path = RouteDescription::new(Method::GET, "/items/{user_agent}")
            .parameter(Parameter::path("user_agent", Schema::string()))
            .parameter(Parameter::path("x_tag", Schema::array(Schema::string())).optional());

        assert_eq!(described(ParameterLocation::Path), path);
    }

    #[test]
    fn declares_header_fields_with_hyphens_like_fastapi() {
        let header = RouteDescription::new(Method::GET, "/items/{user_agent}")
            .parameter(Parameter::header("user-agent", Schema::string()))
            .parameter(Parameter::header("x-tag", Schema::array(Schema::string())).optional());

        assert_eq!(described(ParameterLocation::Header), header);
    }
}
