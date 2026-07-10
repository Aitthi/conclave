from .user import User


def authenticate(u: User) -> bool:
    return len(u.name) > 0


def revoke(u: User) -> bool:
    return authenticate(u)
