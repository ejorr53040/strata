class Base:
    def run(self):
        return self.step()

    def step(self):
        return 0


class Sub(Base):
    def step(self):
        return 1
