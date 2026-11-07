"""Focused tests for the item-boundary host manifest comparison."""
import unittest

from manifest_gate import compare


class ManifestGateTest(unittest.TestCase):
    def test_signature_changes_do_not_hide_docs_or_name_changes(self):
        old = 'Host\tstack\tfn stack() -> EngineStack\t"Read stack"\n'
        new = 'Host\tstack\tfn stack() -> ItemStack\t"Read stack"\n'
        self.assertEqual(compare(old, new, {}), [])
        self.assertTrue(compare(old, new.replace('Read stack', 'Write stack'), {}))
        self.assertTrue(compare(old, new.replace('Host\tstack', 'Host\tother'), {}))

    def test_exact_exception_is_required_for_a_removed_method(self):
        old = 'Host\ttags\tfn tags()\t"Tags"\n'
        self.assertTrue(compare(old, '', {}))
        exception = {'Host::tags': {'old': old.strip(), 'new': None}}
        self.assertEqual(compare(old, '', exception), [])
        self.assertTrue(compare(old.replace('"Tags"', '"Changed"'), '', exception))

    def test_unused_exception_fails(self):
        line = 'Host\tstack\tfn stack()\t"Stack"'
        exception = {'Host::stack': {'old': line, 'new': line}}
        self.assertTrue(compare(line, line, exception))


if __name__ == '__main__':
    unittest.main()
