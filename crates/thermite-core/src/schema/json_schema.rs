use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::{Field, Kind, Presence, Schema};

pub(crate) type JsonSchema = Map<String, Value>;

// the schemas a document refers to by name; None marks one the document itself adds
#[derive(Debug, Default)]
pub(crate) struct Components<'schema> {
    schemas: BTreeMap<&'schema str, (Option<&'schema Schema>, JsonSchema)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NameConflict(pub(crate) String);

impl<'schema> Components<'schema> {
    pub(crate) fn add_built_in(
        &mut self,
        name: &'schema str,
        schema: JsonSchema,
    ) -> Result<(), NameConflict> {
        if self.schemas.contains_key(name) {
            return Err(NameConflict(name.to_owned()));
        }
        self.schemas.insert(name, (None, schema));
        Ok(())
    }

    // like FastAPI, sorted by name
    pub(crate) fn into_json(self) -> JsonSchema {
        self.schemas
            .into_iter()
            .map(|(name, (_, schema))| (name.to_owned(), Value::Object(schema)))
            .collect()
    }

    fn model(
        &mut self,
        name: &'schema str,
        model: &'schema Schema,
        fields: &'schema [Field],
    ) -> Result<JsonSchema, NameConflict> {
        match self.schemas.get(name) {
            Some((Some(known), _)) if *known == model => {}
            Some(_) => return Err(NameConflict(name.to_owned())),
            None => {
                let schema = object(fields, Some(name), self)?;
                self.schemas.insert(name, (Some(model), schema));
            }
        }
        Ok(entries([(
            "$ref",
            format!("#/components/schemas/{name}").into(),
        )]))
    }
}

// keys are sorted the way pydantic sorts them, except the fields under `properties`
impl Schema {
    pub(crate) fn json_schema<'schema>(
        &'schema self,
        components: &mut Components<'schema>,
    ) -> Result<JsonSchema, NameConflict> {
        Ok(match &self.0 {
            Kind::String => of_type("string"),
            Kind::Integer => of_type("integer"),
            Kind::Number => of_type("number"),
            Kind::Boolean => of_type("boolean"),
            Kind::Array(items) => entries([
                ("items", Value::Object(items.json_schema(components)?)),
                ("type", "array".into()),
            ]),
            Kind::Object { name: None, fields } => object(fields, None, components)?,
            Kind::Object {
                name: Some(name),
                fields,
            } => components.model(name, self, fields)?,
        })
    }
}

impl Field {
    pub(crate) fn json_schema<'schema>(
        &'schema self,
        components: &mut Components<'schema>,
    ) -> Result<JsonSchema, NameConflict> {
        let schema = self.schema.json_schema(components)?;
        let mut schema = match self.presence {
            Presence::Optional => {
                entries([("anyOf", json!([Value::Object(schema), { "type": "null" }]))])
            }
            Presence::Required | Presence::Default(_) => schema,
        };
        // like pydantic, a reference stands for the model, which carries its own title
        if !schema.contains_key("$ref") {
            schema.insert("title".to_owned(), title(&self.name).into());
        }
        if let Presence::Default(value) = &self.presence {
            schema.insert("default".to_owned(), value.clone());
        }
        schema.sort_keys();
        Ok(schema)
    }
}

// pydantic's default title: `item_id` becomes `Item Id`, through Python's str.title()
pub(crate) fn title(name: &str) -> String {
    let mut title = String::with_capacity(name.len());
    let mut follows_letter = false;
    for character in name.chars() {
        if follows_letter {
            title.extend(character.to_lowercase());
        } else {
            title.extend(character.to_uppercase());
        }
        follows_letter = character.is_alphabetic();
    }
    title.replace('_', " ")
}

fn object<'schema>(
    fields: &'schema [Field],
    title: Option<&str>,
    components: &mut Components<'schema>,
) -> Result<JsonSchema, NameConflict> {
    let mut properties = JsonSchema::new();
    for field in fields {
        properties.insert(
            field.name.clone(),
            Value::Object(field.json_schema(components)?),
        );
    }
    let required: Vec<Value> = fields
        .iter()
        .filter(|field| field.is_required())
        .map(|field| field.name.as_str().into())
        .collect();
    let mut schema = entries([("properties", Value::Object(properties))]);
    if !required.is_empty() {
        schema.insert("required".to_owned(), Value::Array(required));
    }
    if let Some(title) = title {
        schema.insert("title".to_owned(), title.into());
    }
    schema.insert("type".to_owned(), "object".into());
    Ok(schema)
}

fn of_type(name: &str) -> JsonSchema {
    entries([("type", name.into())])
}

pub(crate) fn entries<const N: usize>(pairs: [(&str, Value); N]) -> JsonSchema {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn document(schema: &Schema) -> (Value, Value) {
        let mut components = Components::default();
        let json = schema.json_schema(&mut components).unwrap();
        (Value::Object(json), Value::Object(components.into_json()))
    }

    #[test]
    fn titles_names_the_way_pydantic_does() {
        assert_eq!(title("item_id"), "Item Id");
        assert_eq!(title("x-token"), "X-Token");
        assert_eq!(title("HTTP_version2b"), "Http Version2B");
    }

    #[test]
    fn writes_scalars_and_arrays_in_place() {
        let (schema, components) = document(&Schema::array(Schema::array(Schema::number())));

        assert_eq!(
            schema,
            json!({"items": {"items": {"type": "number"}, "type": "array"}, "type": "array"})
        );
        assert_eq!(components, json!({}));
    }

    #[test]
    fn describes_fields_by_presence() {
        let (schema, _) = document(&Schema::object([
            Field::new("name", Schema::string()),
            Field::new("size", Schema::integer()).optional(),
            Field::new("tags", Schema::array(Schema::boolean())).with_default(json!([true])),
        ]));

        assert_eq!(
            schema,
            json!({
                "properties": {
                    "name": {"title": "Name", "type": "string"},
                    "size": {"anyOf": [{"type": "integer"}, {"type": "null"}], "title": "Size"},
                    "tags": {
                        "default": [true],
                        "items": {"type": "boolean"},
                        "title": "Tags",
                        "type": "array"
                    }
                },
                "required": ["name"],
                "type": "object"
            })
        );
    }

    #[test]
    fn refers_to_each_model_by_name_and_describes_it_once() {
        let tag = Schema::model("Tag", [Field::new("label", Schema::string())]);
        let item = Schema::model(
            "Item",
            [
                Field::new("main", tag.clone()),
                Field::new("other", tag.clone()).optional(),
                Field::new("all", Schema::array(tag)).with_default(json!([])),
            ],
        );

        let (schema, components) = document(&Schema::array(item));

        assert_eq!(
            schema,
            json!({"items": {"$ref": "#/components/schemas/Item"}, "type": "array"})
        );
        assert_eq!(
            components,
            json!({
                "Item": {
                    "properties": {
                        "main": {"$ref": "#/components/schemas/Tag"},
                        "other": {
                            "anyOf": [{"$ref": "#/components/schemas/Tag"}, {"type": "null"}],
                            "title": "Other"
                        },
                        "all": {
                            "default": [],
                            "items": {"$ref": "#/components/schemas/Tag"},
                            "title": "All",
                            "type": "array"
                        }
                    },
                    "required": ["main"],
                    "title": "Item",
                    "type": "object"
                },
                "Tag": {
                    "properties": {"label": {"title": "Label", "type": "string"}},
                    "required": ["label"],
                    "title": "Tag",
                    "type": "object"
                }
            })
        );
    }

    #[test]
    fn two_different_models_cannot_share_a_name() {
        let first = Schema::model("Item", [Field::new("name", Schema::string())]);
        let second = Schema::model("Item", [Field::new("name", Schema::integer())]);
        let mut components = Components::default();

        first.json_schema(&mut components).unwrap();
        let conflict = second.json_schema(&mut components).err();

        assert_eq!(conflict, Some(NameConflict("Item".to_owned())));
    }

    #[test]
    fn a_built_in_schema_cannot_share_a_name_with_a_model() {
        let model = Schema::model("ValidationError", []);
        let mut components = Components::default();

        model.json_schema(&mut components).unwrap();
        let conflict = components.add_built_in("ValidationError", of_type("object"));

        assert_eq!(conflict, Err(NameConflict("ValidationError".to_owned())));
    }
}
