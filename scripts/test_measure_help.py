"""Contract for the topic paragraph consumed by the help-size tool."""
import runpy
import unittest
from pathlib import Path


topic_names = runpy.run_path(str(Path(__file__).with_name("measure-help")))["topic_names"]


class TopicNamesTest(unittest.TestCase):
    def test_only_topic_paragraph_is_queried(self):
        page = b"""Intro: not a topic
Topics (vork help TOPIC [--all]):
light-paths: paths: with a colon in the description
field: exact lookup
std: builders

appendix: ordinary prose with a valid-looking topic prefix
More prose: another colon
"""
        self.assertEqual(topic_names(page), ["light-paths", "field", "std"])

    def test_final_topic_needs_no_trailing_paragraph(self):
        page = b"Topics (vork help TOPIC [--all]):\ngeometry: meshes\n"
        self.assertEqual(topic_names(page), ["geometry"])


if __name__ == "__main__":
    unittest.main()
