"""Git setup for repositories created by tests, never for the project checkout."""
import os
import subprocess


def isolated_git_env(**overrides: str) -> dict[str, str]:
    """Keep fixture Git subprocesses independent of the caller's config and checkout."""
    return {**{key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
            "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1", **overrides}


def git_in(folder):
    """Return a Git command bound to the fixture directory."""
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=folder, text=True, stderr=subprocess.PIPE,
                                       env=isolated_git_env()).strip()
    return git


def decoy_git_env(folder):
    """Simulate a caller with its own clean, disposable checkout selected."""
    folder.mkdir()
    git = git_in(folder)
    git("init", "-q", "-b", "main")
    git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "-qm", "decoy", "--allow-empty")
    return isolated_git_env(GIT_DIR=str(folder / ".git"), GIT_WORK_TREE=str(folder))
