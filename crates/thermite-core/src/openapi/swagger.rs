use serde_json::Value;

// FastAPI's Swagger UI page, loaded from the same CDN
pub(super) fn page(title: &str, openapi_path: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html>
<head>
<link type="text/css" rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
<title>{title} - Swagger UI</title>
</head>
<body>
<div id="swagger-ui">
</div>
<script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
<script>
const ui = SwaggerUIBundle({{
    url: {url},
    dom_id: "#swagger-ui",
    layout: "BaseLayout",
    deepLinking: true,
    showExtensions: true,
    showCommonExtensions: true,
    presets: [
        SwaggerUIBundle.presets.apis,
        SwaggerUIBundle.SwaggerUIStandalonePreset
    ],
}})
</script>
</body>
</html>"##,
        title = escape_html(title),
        url = script_string(openapi_path),
    )
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

// a JSON string is a JavaScript string, and an escaped `<` cannot close the script element
fn script_string(text: &str) -> String {
    Value::from(text).to_string().replace('<', "\\u003c")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_swagger_ui_at_the_document() {
        let page = page("Todos", "/api/openapi.json");

        assert!(page.contains("<title>Todos - Swagger UI</title>"));
        assert!(page.contains(r#"url: "/api/openapi.json","#));
    }

    #[test]
    fn escapes_the_title_and_the_path() {
        let page = page("<Tom & Jerry's \"API\">", "/</script>\"");

        assert!(
            page.contains("<title>&lt;Tom &amp; Jerry&#x27;s &quot;API&quot;&gt; - Swagger UI")
        );
        assert!(page.contains(r#"url: "/\u003c/script>\"","#));
    }
}
