# the FastAPI side of the comparison; app.rs next to it is the same app in thermite
from fastapi import FastAPI
from fastapi.responses import PlainTextResponse
from pydantic import BaseModel

app = FastAPI()


class Item(BaseModel):
    name: str
    price: float
    tags: list[str]


class ItemView(BaseModel):
    item_id: int
    q: str | None
    limit: int


class Count(BaseModel):
    count: int


class Message(BaseModel):
    message: str


@app.get("/plaintext", response_class=PlainTextResponse)
async def plaintext() -> str:
    return "Hello, World!"


@app.get("/json")
async def json() -> Message:
    return Message(message="Hello, World!")


@app.get("/items/{item_id}")
async def read_item(item_id: int, q: str | None = None, limit: int = 10) -> ItemView:
    return ItemView(item_id=item_id, q=q, limit=limit)


@app.post("/items")
async def create_item(item: Item) -> Item:
    return item


@app.post("/items/batch")
async def create_items(items: list[Item]) -> Count:
    return Count(count=len(items))
