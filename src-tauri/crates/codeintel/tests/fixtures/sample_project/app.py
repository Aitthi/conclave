class Cat:
    def __init__(self, name):
        self.name = name

    def meow(self):
        return "meow"


def main():
    print(Cat("nyan").meow())
