import multiprocessing

from thermite import App

app = App()

if multiprocessing.parent_process() is not None:
    raise ValueError("this app cannot start in a worker")
