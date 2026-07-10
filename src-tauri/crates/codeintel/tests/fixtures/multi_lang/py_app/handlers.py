from .auth import authenticate
from .user import User, new_user


def login(name: str):
    u = new_user(1, name)
    return u if authenticate(u) else None


def whoami(u: User) -> str:
    return u.name
