import asyncio
import os

from thermite import App

app = App()


@app.get("/pid")
async def pid():
    # long enough for concurrent requests to spread over the workers
    await asyncio.sleep(0.02)
    return os.getpid()


@app.get("/blocking")
def blocking():
    return os.getpid()


@app.get("/slow")
async def slow(seconds: float):
    await asyncio.sleep(seconds)
    return "finished"
