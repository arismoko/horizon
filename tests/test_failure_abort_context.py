"""A test that dies without its own panic still says why: the runtime's abort and other threads' panics."""
import contextlib
import io
import unittest

from tools.ci import ignored_tests as ci


def render(output):
    with contextlib.redirect_stdout(io.StringIO()) as log:
        ci.failure_diagnostics(output)
    return log.getvalue()


class AbortContext(unittest.TestCase):
    def test_a_stack_overflow_is_named(self):
        log = render("test game::capture ... FAILED\n"
                     "\n"
                     "thread 'game::capture' has overflowed its stack\n"
                     "fatal runtime error: stack overflow, aborting\n")
        self.assertIn("  failed test: game::capture\n  process aborted: stack overflow\n", log)
        self.assertNotIn("panic context missing", log)

    def test_a_server_threads_panic_and_the_abort_after_it_are_shown_without_payload(self):
        log = render("test game::capture ... FAILED\n"
                     "thread 'Server thread' panicked at private.rs:1:1:\n"
                     "PRIVATE world name SecretWorld\n"
                     "thread 'Server thread' panicked while processing panic. aborting.\n")
        self.assertIn("  process aborted: a panic while panicking\n"
                      "  on a non-test thread:\n"
                      "  panic location withheld (not a bounded tracked Rust location)\n"
                      "  panic reason: custom panic; payload withheld (resource privacy)\n", log)
        self.assertNotIn("SecretWorld", log)

    def test_nothing_known_stays_missing(self):
        self.assertIn("panic context missing from captured output",
                      render("test game::capture ... FAILED\nsomething else\n"))


if __name__ == "__main__":
    unittest.main()
