import thermite
from thermite import App


def test_the_package_exports_its_public_names():
    assert thermite.__all__ == [
        "App",
        "Body",
        "Cookie",
        "Header",
        "Path",
        "Query",
        "ResponseValidationError",
        "RouteError",
    ]


def test_a_new_app_documents_no_routes():
    assert App().openapi()["paths"] == {}


def test_every_method_decorator_registers_its_method():
    app = App()
    for method in ["get", "post", "put", "delete", "patch", "head", "options", "trace"]:

        @getattr(app, method)("/thing")
        def handler():
            pass

    assert sorted(app.openapi()["paths"]["/thing"]) == [
        "delete",
        "get",
        "head",
        "options",
        "patch",
        "post",
        "put",
        "trace",
    ]


def test_the_decorator_returns_the_handler_unchanged():
    def handler():
        pass

    assert App().get("/thing")(handler) is handler


def test_the_handler_name_gives_the_summary_and_a_callable_object_its_class_name():
    class Reader:
        def __call__(self):
            pass

    app = App()
    app.get("/function")(test_a_new_app_documents_no_routes)
    app.get("/object")(Reader())

    paths = app.openapi()["paths"]
    assert paths["/function"]["get"]["summary"] == "Test A New App Documents No Routes"
    assert paths["/object"]["get"]["summary"] == "Reader"


def test_async_and_plain_handlers_are_both_accepted():
    app = App()

    @app.get("/plain")
    def plain():
        pass

    @app.get("/async")
    async def asynchronous():
        pass

    assert sorted(app.openapi()["paths"]) == ["/async", "/plain"]
