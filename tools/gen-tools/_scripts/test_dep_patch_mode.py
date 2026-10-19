#!/usr/bin/env python3
"""patch-local / patch-git: version lines stay, one crates.io patch.

No network and no monorepo writes. The temp workspace is the only Cargo.toml
these tests rewrite.
"""

from __future__ import annotations

import importlib.util
import io
import shutil
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

_SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(_SCRIPTS))

import dep_common  # noqa: E402


def _load_switch():
    path = _SCRIPTS / "dep-switch.py"
    spec = importlib.util.spec_from_file_location("dep_switch", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


dep_switch = _load_switch()
from dep_common import (  # noqa: E402
    PATCH_MODE_MAP,
    build_entry,
    build_ws_dep_entry,
    git_rev,
    load_matrix,
    drop_unreferenced_git_patches,
    git_dependency_urls,
    lint_unused_patches,
    pin_floating_git_deps,
    pkg_to_matrix,
    registry_version_literal,
    same_source_git_patches,
)

COS_REV = "e20646e2d9a9c4f93e993eb0656afa795f6e387a"
PLUS_REV = "f7ef1aaafd368d811f268cf47360a7d808e74102"
MULTI_REV = "cbf6c519e16840ba2454553ff117f482d539b4cd"


def _crate(stable, local, url, rev=None, package=None, fork=None, aliases=None):
    info = {"stable": stable, "local": local, "git": {"url": url}}
    if rev is not None:
        info["git"]["rev"] = rev
    if package:
        info["package"] = package
    if fork:
        info["fork_version"] = fork
    if aliases:
        info["dep_aliases"] = aliases
    return info


class RegistryAndRevTests(unittest.TestCase):
    def test_fork_version_is_exact_even_when_stable_differs(self):
        info = {"stable": "3.0.4", "fork_version": "4.0.0-zk"}
        self.assertEqual(registry_version_literal(info), "=4.0.0-zk")
        entry = build_ws_dep_entry(
            "cosmwasm-std", info, "registry", "dao-contracts/Cargo.toml",
            {"features": ["std", "stargate"]},
        )
        self.assertIn('version = "=4.0.0-zk"', entry)
        self.assertIn('"stargate"', entry)
        self.assertNotIn("git =", entry)

    def test_plain_stable_stays_a_caret_requirement(self):
        self.assertEqual(registry_version_literal({"stable": "3.0.1"}), "3.0.1")

    def test_prerelease_stable_is_exact_without_fork_version(self):
        self.assertEqual(registry_version_literal({"stable": "3.0.0-rc"}), "=3.0.0-rc")

    def test_git_rev_writes_full_sha_and_not_a_branch(self):
        info = _crate("3.0.4", "cosmwasm/packages/std",
                      "https://github.com/permissionlessweb/cosmwasm", COS_REV)
        info["git"]["branch"] = "mvp"
        entry = build_entry("cosmwasm-std", info, "git-rev", "dao-contracts/Cargo.toml")
        self.assertEqual(
            entry,
            f'{{ git = "https://github.com/permissionlessweb/cosmwasm", rev = "{COS_REV}" }}',
        )
        self.assertNotIn("branch", entry)

    def test_short_sha_is_rejected_and_does_not_fall_through(self):
        info = {"git": {"url": "https://example.com/fork", "rev": "e20646e2", "commit": COS_REV}}
        self.assertIsNone(git_rev(info))
        self.assertIsNone(build_entry("cosmwasm-std", info, "git-rev", "dao-contracts/Cargo.toml"))

    def test_commit_field_is_accepted_when_rev_is_absent(self):
        info = {"git": {"url": "https://example.com/fork", "commit": COS_REV, "branch": "cw-svg"}}
        self.assertEqual(git_rev(info), COS_REV)

    def test_clone_cw_multi_test_keeps_its_own_package_name(self):
        crates = load_matrix()
        clone = crates["clone-cw-multi-test"]
        self.assertEqual(clone["package"], "cw-multi-test")
        self.assertEqual(registry_version_literal(clone), "=3.0.0-rc")
        self.assertNotIn("abstract-cw-multi-test", clone.get("dep_aliases", []))
        lookup = pkg_to_matrix(crates)
        self.assertEqual(lookup["clone-cw-multi-test"][0], "clone-cw-multi-test")
        self.assertEqual(lookup["abstract-cw-multi-test"][0], "cw-multi-test_zk")
        self.assertEqual(lookup["cw-multi-test"][0], "cw-multi-test_zk")

    def test_patch_modes_are_one_replacement_each(self):
        self.assertEqual(PATCH_MODE_MAP["patch-local"], ("registry", "local"))
        self.assertEqual(PATCH_MODE_MAP["patch-git"], ("registry", "git-rev"))
        self.assertNotIn("dev", PATCH_MODE_MAP)


class RewriteTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self._old_common = dep_common.REPO_ROOT
        self._old_switch = dep_switch.REPO_ROOT
        dep_common.REPO_ROOT = self.root
        dep_switch.REPO_ROOT = self.root
        ws = self.root / "dao-contracts"
        member = ws / "packages" / "dao-testing"
        member.mkdir(parents=True)
        (ws / "Cargo.toml").write_text(
            "\n".join([
                "[workspace]",
                'members = ["packages/*"]',
                "",
                "[workspace.dependencies]",
                'cosmwasm-std = { git = "https://github.com/permissionlessweb/cosmwasm", features = ["std"] }',
                'cw-multi-test = { package = "abstract-cw-multi-test", git = "https://github.com/permissionlessweb/cw-multi-test-fork" }',
                'cw-utils = { version = "3.0.0" }',
                "",
                "[patch.'https://github.com/permissionlessweb/cosmwasm']",
                'cosmwasm-std = { path = "../cosmwasm/packages/std" }',
                "",
            ])
        )
        (member / "Cargo.toml").write_text(
            "\n".join([
                "[package]",
                'name = "dao-testing"',
                'version = "0.1.0"',
                "",
                "[dependencies]",
                'cw1-whitelist = { git = "https://github.com/permissionlessweb/cw-plus" }',
                "",
            ])
        )
        self.crates = {
            "cosmwasm-std": _crate(
                "3.0.4", "cosmwasm/packages/std",
                "https://github.com/permissionlessweb/cosmwasm", COS_REV, fork="4.0.0-zk",
            ),
            "cw-multi-test_zk": _crate(
                "3.0.0-rc", "cw-multi-test-fork",
                "https://github.com/permissionlessweb/cw-multi-test-fork", MULTI_REV,
                package="abstract-cw-multi-test", aliases=["cw-multi-test"],
            ),
            "cw1-whitelist": _crate(
                "3.0.0", "cw-plus/contracts/cw1-whitelist",
                "https://github.com/permissionlessweb/cw-plus", PLUS_REV,
            ),
            "cw-utils": _crate(
                "3.0.0", "cw-minus/packages/cw-utils",
                "https://github.com/permissionlessweb/cw-minus", rev="c5c680dd",
            ),
        }

    def tearDown(self):
        dep_common.REPO_ROOT = self._old_common
        dep_switch.REPO_ROOT = self._old_switch
        self.tmp.cleanup()

    def test_patch_git_inserts_one_rev_patch_per_declared_crate(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            changed = dep_switch.rewrite_patches(
                "dao-contracts/Cargo.toml", self.crates, "git-rev", ensure=True,
            )
        self.assertTrue(changed)
        text = (self.root / "dao-contracts" / "Cargo.toml").read_text()
        self.assertIn("[patch.crates-io]", text)
        self.assertIn(
            f'cosmwasm-std = {{ git = "https://github.com/permissionlessweb/cosmwasm", rev = "{COS_REV}" }}',
            text,
        )
        self.assertIn(
            f'abstract-cw-multi-test = {{ git = "https://github.com/permissionlessweb/cw-multi-test-fork", rev = "{MULTI_REV}" }}',
            text,
        )
        self.assertIn(
            f'cw1-whitelist = {{ git = "https://github.com/permissionlessweb/cw-plus", rev = "{PLUS_REV}" }}',
            text,
        )
        patch_body = text.split("[patch.crates-io]", 1)[1]
        self.assertNotIn("\ncw-multi-test =", patch_body)
        self.assertNotIn("branch =", text)
        self.assertIn("not a full 40-hex sha", buf.getvalue())
        self.assertNotIn("cw-utils = { git", text)

        with redirect_stdout(io.StringIO()):
            dep_switch.remove_git_url_patches("dao-contracts/Cargo.toml", self.crates)
        text = (self.root / "dao-contracts" / "Cargo.toml").read_text()
        self.assertNotIn("[patch.'https://github.com/permissionlessweb/cosmwasm']", text)
        self.assertNotIn("path =", text)

    def test_double_quoted_git_url_patch_is_removed(self):
        path = self.root / "dao-contracts" / "Cargo.toml"
        path.write_text(
            "\n".join([
                "[workspace]",
                'members = ["packages/*"]',
                "",
                "[workspace.dependencies]",
                'cw-orch = { version = "0.30.0" }',
                "",
                "[patch.crates-io]",
                'cw-orch = { git = "https://github.com/permissionlessweb/cw-orchestrator", rev = "' + COS_REV + '" }',
                "",
                '[patch."https://github.com/permissionlessweb/cw-orchestrator"]',
                'cw-orch = { path = "../cw-orchestrator/cw-orch" }',
                "",
            ])
        )
        crates = {
            "cw-orch": _crate(
                "0.30.0", "cw-orchestrator/cw-orch",
                "https://github.com/permissionlessweb/cw-orchestrator", COS_REV,
            ),
        }
        with redirect_stdout(io.StringIO()):
            dep_switch.remove_git_url_patches("dao-contracts/Cargo.toml", crates)
        text = path.read_text()
        self.assertNotIn('[patch."https://github.com/permissionlessweb/cw-orchestrator"]', text)
        self.assertNotIn("path =", text)
        self.assertIn("[patch.crates-io]", text)

    def test_patch_local_writes_paths_for_the_same_declarations(self):
        with redirect_stdout(io.StringIO()):
            dep_switch.rewrite_patches(
                "dao-contracts/Cargo.toml", self.crates, "local", ensure=True,
            )
        text = (self.root / "dao-contracts" / "Cargo.toml").read_text()
        self.assertIn('cosmwasm-std = { path = "../cosmwasm/packages/std" }', text)
        self.assertIn('abstract-cw-multi-test = { path = "../cw-multi-test-fork" }', text)
        self.assertIn('cw1-whitelist = { path = "../cw-plus/contracts/cw1-whitelist" }', text)
        self.assertNotIn("rev =", text.split("[patch.crates-io]", 1)[1])

    def test_rel_path_does_not_follow_a_symlink_out_of_the_monorepo(self):
        outside = Path(tempfile.mkdtemp())
        try:
            link_parent = self.root / "zcash"
            link_parent.mkdir()
            (link_parent / "halo2").symlink_to(outside)
            (self.root / "cosmwasm").mkdir()
            got = dep_common.rel_path("cosmwasm/Cargo.toml", "zcash/halo2/halo2_gadgets")
        finally:
            shutil.rmtree(outside, ignore_errors=True)
        self.assertEqual(got, "../zcash/halo2/halo2_gadgets")

    def test_workspace_dependencies_are_not_member_deps(self):
        lines = [
            "[workspace.dependencies]",
            'cosmwasm-std = { version = "1", features = ["std"] }',
            "[dependencies]",
            'serde = "1"',
            "[target.'cfg(unix)'.dependencies]",
            'libc = "0.2"',
        ]
        self.assertFalse(dep_switch._is_in_dep_section(lines, 1))
        self.assertEqual(dep_switch._is_in_dep_section(lines, 3), "dependencies")
        self.assertEqual(
            dep_switch._is_in_dep_section(lines, 5),
            "target.'cfg(unix)'.dependencies",
        )


class PinRevTests(unittest.TestCase):
    def _crates(self):
        return {
            "cosmos-sdk-proto": _crate(
                "0.28.0", "cosmos-rust/cosmos-sdk-proto",
                "https://github.com/permissionlessweb/cosmos-rust", COS_REV,
            ),
        }

    def test_branch_becomes_the_matrix_rev(self):
        text = (
            'cosmos-sdk-proto = { git = "https://github.com/permissionlessweb/cosmos-rust", '
            'branch = "zk-mvp", package = "cosmos-sdk-proto" }\n'
        )
        new, changes, warnings = pin_floating_git_deps(text, self._crates())
        self.assertEqual(warnings, [])
        self.assertEqual(len(changes), 1)
        self.assertIn(f'rev = "{COS_REV}"', new)
        self.assertNotIn("branch", new)
        again, changes2, _warnings = pin_floating_git_deps(new, self._crates())
        self.assertEqual(changes2, [])
        self.assertEqual(again, new)

    def test_unknown_branch_is_left_alone(self):
        text = 'other = { git = "https://github.com/permissionlessweb/other", branch = "main" }\n'
        new, changes, warnings = pin_floating_git_deps(text, self._crates())
        self.assertEqual(new, text)
        self.assertEqual(changes, [])
        self.assertEqual(warnings, [])

    def test_same_source_git_patch_is_reported(self):
        text = "\n".join([
            "[patch.'https://github.com/permissionlessweb/cosmos-rust']",
            'cosmos-sdk-proto = { git = "https://github.com/permissionlessweb/cosmos-rust", rev = "' + COS_REV + '" }',
            "",
        ])
        self.assertEqual(len(same_source_git_patches(text)), 1)
        direct = (
            'cosmos-sdk-proto = { git = "https://github.com/permissionlessweb/cosmos-rust", '
            'rev = "' + COS_REV + '" }\n'
        )
        self.assertEqual(same_source_git_patches(direct), [])


class LintPatchTests(unittest.TestCase):
    def test_unused_duplicate_paths_are_removed_and_order_stays(self):
        root = Path(tempfile.mkdtemp())
        try:
            target = root / "cw-plus" / "packages" / "cw3"
            target.mkdir(parents=True)
            other = root / "cw-plus" / "packages" / "cw20"
            other.mkdir()
            text = "\n".join([
                "[workspace]",
                'members = ["packages/*"]',
                "",
                "[patch.crates-io]",
                'cw20 = { path = "cw-plus/packages/cw20" }',
                'cw3 = { path = "cw-plus/packages/cw3" }',
                "",
                "[patch.'https://github.com/permissionlessweb/cw-plus']",
                'cw1 = { path = "cw-plus/packages/cw1" }',
                'cw3 = { path = "cw-plus/packages/cw3" }',
                "",
                "[profile.release]",
                "opt-level = 3",
                "",
            ])
            source = str(target.resolve())
            new, removed, dropped, unmatched = lint_unused_patches(
                text, root, [("cw3", source), ("cw3", source)]
            )
        finally:
            shutil.rmtree(root, ignore_errors=True)
        self.assertEqual(unmatched, [])
        self.assertEqual(dropped, [])
        self.assertEqual(len(removed), 2)
        self.assertNotIn("cw3", new)
        self.assertIn('cw20 = { path = "cw-plus/packages/cw20" }', new)
        self.assertIn('cw1 = { path = "cw-plus/packages/cw1" }', new)
        self.assertLess(new.index("cw20"), new.index("cw1"))
        self.assertLess(new.index("[patch.crates-io]"), new.index("[profile.release]"))

    def test_empty_patch_section_and_its_comment_go_away(self):
        root = Path(tempfile.mkdtemp())
        try:
            target = root / "only"
            target.mkdir()
            text = "\n".join([
                "[patch.crates-io]",
                'kept = { path = "only" }',
                "",
                "# remote override",
                "[patch.'https://github.com/permissionlessweb/cw-plus']",
                'cw3 = { path = "only" }',
                "",
            ])
            new, removed, dropped, unmatched = lint_unused_patches(
                text, root, [("cw3", str(target.resolve()))]
            )
        finally:
            shutil.rmtree(root, ignore_errors=True)
        self.assertEqual(unmatched, [])
        self.assertEqual(removed, ["cw3 from [https://github.com/permissionlessweb/cw-plus]"])
        self.assertIn("https://github.com/permissionlessweb/cw-plus", dropped)
        self.assertNotIn("remote override", new)
        self.assertIn("kept", new)


class GitPatchRetireTests(unittest.TestCase):
    def test_patch_tables_do_not_count_as_git_dependencies(self):
        text = "\n".join([
            'cosmwasm-std = { version = "=4.0.0-zk" }',
            "",
            "[patch.crates-io]",
            'cosmos-sdk-proto = { git = "https://github.com/permissionlessweb/cosmos-rust", rev = "abc" }',
            "",
        ])
        self.assertEqual(git_dependency_urls(text), set())

    def test_unreferenced_git_section_and_its_comment_go_away(self):
        text = "\n".join([
            "[patch.crates-io]",
            'cw20 = { path = "../cw-plus/packages/cw20" }',
            "",
            "# Unify git edges onto the sibling checkout.",
            "[patch.'https://github.com/permissionlessweb/cw-plus']",
            'cw20 = { path = "../cw-plus/packages/cw20" }',
            "",
            "[patch.'https://github.com/permissionlessweb/cosmwasm']",
            'cosmwasm-std = { path = "../cosmwasm/packages/std" }',
            "",
        ])
        new, dropped = drop_unreferenced_git_patches(
            text, {"https://github.com/permissionlessweb/cosmwasm"}
        )
        self.assertEqual(dropped, ["https://github.com/permissionlessweb/cw-plus"])
        self.assertNotIn("[patch.'https://github.com/permissionlessweb/cw-plus']", new)
        self.assertNotIn("Unify git edges", new)
        self.assertIn("[patch.'https://github.com/permissionlessweb/cosmwasm']", new)
        self.assertIn('cw20 = { path = "../cw-plus/packages/cw20" }', new)

    def test_a_remaining_git_dependency_keeps_its_section(self):
        text = "\n".join([
            'cw-utils = { git = "https://github.com/permissionlessweb/cw-minus", rev = "abc" }',
            "",
            "[patch.'https://github.com/permissionlessweb/cw-minus']",
            'cw-utils = { path = "../cw-minus/packages/cw-utils" }',
            "",
        ])
        urls = git_dependency_urls(text)
        new, dropped = drop_unreferenced_git_patches(text, urls)
        self.assertEqual(dropped, [])
        self.assertIn("cw-minus", new)


class UnusedPatchWarningTests(unittest.TestCase):
    def test_cargo_note_lines_without_warning_prefix_count(self):
        stderr = "\n".join([
            "warning: Patch `cw-storage-plus v3.0.1 (/tmp/cw-storage-plus)` was not used in the crate graph.",
            "Patch `cw2 v3.0.0 (/tmp/cw2)` was not used in the crate graph.",
        ])
        from dep_common import unused_patch_warnings
        self.assertEqual(
            unused_patch_warnings(stderr),
            [("cw-storage-plus", "/tmp/cw-storage-plus"), ("cw2", "/tmp/cw2")],
        )


if __name__ == "__main__":
    unittest.main()
