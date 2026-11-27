import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from tests.git_fixture import isolated_git_env
from tools import check_addresses, merge_base


def strings(source):
    """The string literals a source file keeps once its #[cfg(test)] items are dropped."""
    literals, mask = check_addresses.tokenize(source)
    ranges, files = check_addresses.test_items(source, mask)
    return [s for offset, s in literals if not any(a <= offset < b for a, b in ranges)], files


CONFIG = {
    "homes": ["crates/sunrise-addresses/src/lib.rs"],
    "vanilla": {"textures.minecraft.net": "skins"},
    "not_addresses": {"discord.com": "a webhook host"},
    "allowed": ["crates/sunrise-telemetry/src/lib.rs telemetry.playsunrise.net"],
}


class HostsTests(unittest.TestCase):
    def test_urls_and_whole_host_strings(self):
        self.assertEqual(check_addresses.hosts("https://telemetry.playsunrise.net/v1/report"),
                         ["telemetry.playsunrise.net"])
        self.assertEqual(check_addresses.hosts("stun.l.google.com:19302"), ["stun.l.google.com"])
        self.assertEqual(check_addresses.hosts("wss://user:pw@relay.playsunrise.net:443/x"),
                         ["relay.playsunrise.net"])

    def test_text_translation_keys_and_placeholders_are_not_hosts(self):
        for text in ["Download it at playsunrise.net, then press Play.", "options.online", "mco.info",
                     "https://example.com/a", "https://foo.test", "http://127.0.0.1:8080", "http://192.168.1.1/",
                     "https://map.example.net", "http://192.0.2.7:8100", "vertex.xyz * 2.0"]:
            self.assertEqual(check_addresses.hosts(text), [], text)

    def test_public_ip_in_a_url_counts(self):
        self.assertEqual(check_addresses.hosts("http://8.8.8.8/"), ["8.8.8.8"])


class TokenizeTests(unittest.TestCase):
    def test_comments_are_not_strings(self):
        found, _ = strings('// "https://a.com"\n/* "https://b.com" */ let x = "https://c.com";')
        self.assertEqual(found, ["https://c.com"])

    def test_raw_strings_char_literals_and_lifetimes(self):
        found, _ = strings("fn f<'a>(c: char) { if c == '\"' {} let r = r#\"x \"q\" y\"#; let s = \"z\"; }")
        self.assertEqual(found, ['x "q" y', "z"])

    def test_cfg_test_modules_are_skipped_whatever_their_strings_hold(self):
        source = ('const A: &str = "https://a.com";\n#[cfg(test)]\nmod tests {\n    const R: &str = r#"\n}\n"#;\n'
                  '    const B: &str = "https://b.com";\n}\nconst C: &str = "https://c.com";\n')
        found, _ = strings(source)
        self.assertEqual(found, ["https://a.com", "https://c.com"])

    def test_cfg_test_files_are_named(self):
        _, files = strings('#[cfg(test)]\nmod tests;\n#[cfg(test)]\n#[path = "fetch_tests.rs"]\nmod fetch_tests;\n')
        self.assertEqual(files, ["tests", "fetch_tests.rs"])

    def test_other_cfg_test_items_are_skipped(self):
        found, _ = strings('#[cfg(test)]\nconst T: [&str; 1] = ["https://t.com"];\nconst U: &str = "https://u.com";')
        self.assertEqual(found, ["https://u.com"])


class CheckTests(unittest.TestCase):
    def run_check(self, found, base=None):
        return check_addresses.check(CONFIG, set(found), base)

    def test_home_vanilla_not_addresses_and_allowed_pass(self):
        found = [("crates/sunrise-addresses/src/lib.rs", "playsunrise.net"),
                 ("crates/sunrise-client-render/src/skins.rs", "textures.minecraft.net"),
                 ("crates/sunrise-server/src/discord.rs", "discord.com"),
                 ("crates/sunrise-telemetry/src/lib.rs", "telemetry.playsunrise.net")]
        self.assertEqual(self.run_check(found), ([], []))

    def test_a_new_address_outside_the_home_fails_and_names_the_doc(self):
        problems, _ = self.run_check([("crates/sunrise-client/src/net.rs", "relay.playsunrise.net")])
        self.assertIn("relay.playsunrise.net", problems[0])
        self.assertIn("docs/layers.md#built-in-addresses", problems[0])

    def test_non_address_identity_can_be_scoped_to_its_file(self):
        identity = "crates/verifier/src/lib.rs"
        client = "crates/downloader/src/lib.rs"
        config = dict(CONFIG, not_addresses={f"{identity} github.com": "certificate identity"})
        problems, _ = check_addresses.check(config, {(identity, "github.com"), (client, "github.com")})
        self.assertEqual(len(problems), 1)
        self.assertIn(client, problems[0])

    def test_an_address_that_left_its_file_is_a_note(self):
        problems, notes = self.run_check([])
        self.assertEqual(problems, [])
        self.assertIn("telemetry.playsunrise.net", notes[0])

    def test_the_list_may_not_take_new_hosts_or_grow(self):
        config = dict(CONFIG, allowed=CONFIG["allowed"] + ["crates/x/src/lib.rs relay.playsunrise.net"])
        problems, _ = check_addresses.check(config, set(), {"allowed": CONFIG["allowed"]})
        self.assertTrue(any("relay.playsunrise.net: added" in p for p in problems))
        self.assertTrue(any("grew from 1 to 2" in p for p in problems))

    def test_an_entry_may_move_with_its_code(self):
        moved = {"allowed": ["crates/sunrise-telemetry/src/upload.rs telemetry.playsunrise.net"]}
        config = dict(CONFIG, allowed=moved["allowed"])
        found = {("crates/sunrise-telemetry/src/upload.rs", "telemetry.playsunrise.net")}
        self.assertEqual(check_addresses.check(config, found, {"allowed": CONFIG["allowed"]}), ([], []))


class PullRequestMergeTests(unittest.TestCase):
    def test_a_pull_request_merge_is_checked_against_main_as_merged(self):
        """main allowed a new address after the branch left it: a pull request merged into that main didn't add it."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = isolated_git_env(GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t.invalid",
                                   GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t.invalid")

            def run(*args):
                return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, env=env,
                                      text=True).stdout.strip()

            def config(allowed):
                return json.dumps({"homes": [], "vanilla": {}, "not_addresses": {}, "allowed": allowed})

            run("init", "-q", "-b", "main")
            (root / "tools").mkdir()
            (root / "tools/addresses.json").write_text(config([]))
            (root / "tools/layers.json").write_text('{"crates": {}}')
            (root / "crates/a/src").mkdir(parents=True)
            (root / "crates/a/src/lib.rs").write_text("pub fn a() {}\n")
            run("add", ".")
            run("commit", "-q", "-m", "base")
            base = run("rev-parse", "HEAD")
            run("checkout", "-q", "-b", "work")
            (root / "crates/a/src/lib.rs").write_text("pub fn a() -> u8 { 1 }\n")
            run("commit", "-q", "-am", "unrelated change")
            run("checkout", "-q", "main")
            (root / "crates/x/src").mkdir(parents=True)
            (root / "crates/x/src/lib.rs").write_text('pub const URL: &str = "https://relay.playsunrise.net";\n')
            (root / "tools/addresses.json").write_text(config(["crates/x/src/lib.rs relay.playsunrise.net"]))
            run("add", ".")
            run("commit", "-q", "-m", "main allows an address")
            run("checkout", "-q", "--detach", "main")
            run("merge", "-q", "--no-ff", "-m", "merge work", "work")
            with mock.patch.object(check_addresses, "ROOT", root), mock.patch.object(merge_base, "ROOT", root), \
                    mock.patch("sys.stdout", new_callable=io.StringIO) as out:
                self.assertEqual(check_addresses.main(["--base", base, "--pull-request-merge"]), 0)
                # Against the branch's old base, main's new entry is charged to the pull request.
                self.assertEqual(check_addresses.main(["--base", base]), 1)
            self.assertIn("relay.playsunrise.net", out.getvalue())


class BaseConfigTests(unittest.TestCase):
    def test_the_base_is_where_the_branch_left_main(self):
        """An entry main dropped after the branch left it is still on the branch's base: the branch didn't add it."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = isolated_git_env(GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t.invalid",
                                   GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t.invalid")

            def run(*args):
                subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, env=env)

            def allow(*entries):
                (root / "tools").mkdir(exist_ok=True)
                (root / "tools" / "addresses.json").write_text(json.dumps({"allowed": list(entries)}))

            run("init", "-q", "-b", "main")
            allow("crates/sunrise-telemetry/src/lib.rs telemetry.playsunrise.net")
            run("add", ".")
            run("commit", "-q", "-m", "base")
            run("checkout", "-q", "-b", "work")
            run("commit", "-q", "--allow-empty", "-m", "work")
            run("checkout", "-q", "main")
            allow()
            run("commit", "-q", "-am", "main drops the entry")
            run("checkout", "-q", "work")
            with mock.patch.object(check_addresses, "ROOT", root):
                self.assertEqual(check_addresses.base_config("main"), {"allowed": ["crates/sunrise-telemetry/src/lib.rs telemetry.playsunrise.net"]})


if __name__ == "__main__":
    unittest.main()
