import thermite
from thermite import App


def test_the_package_exports_app():
    assert thermite.__all__ == ["App"]


def test_app_belongs_to_the_package_and_not_the_native_module():
    assert App.__module__ == "thermite"


def test_app_can_be_created():
    assert isinstance(App(), App)
