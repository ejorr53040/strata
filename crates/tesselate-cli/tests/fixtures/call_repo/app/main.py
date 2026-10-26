import app.util as u
from app.util import helper as h
from .models import Base
import requests


def main():
    h()
    u.other()
    requests.get("x")
    b = Base()
    b.run()
