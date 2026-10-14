"""Example shapes module for Saturnite Python ecosystem testing."""


# Enum-like constants
class Colors:
    RED = 1
    GREEN = 2
    BLUE = 3


class Point:
    def __init__(self, x: float, y: float):
        self.x = x
        self.y = y

    def distance(self, other: "Point") -> float:
        dx = self.x - other.x
        dy = self.y - other.y
        return (dx * dx + dy * dy) ** 0.5
