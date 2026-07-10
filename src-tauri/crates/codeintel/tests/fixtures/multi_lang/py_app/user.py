class User:
    def __init__(self, id: int, name: str) -> None:
        self.id = id
        self.name = name


def new_user(id: int, name: str) -> User:
    return User(id, name)


def unused_helper() -> int:
    return 42
