"""THROWAWAY: deliberately failing test to prove a failing check blocks merge (#170). Never merge."""
import unittest


class Proof(unittest.TestCase):
    def test_deliberate_failure(self):
        self.fail("deliberate failure for the #170 block-on-failure proof")


if __name__ == "__main__":
    unittest.main()
