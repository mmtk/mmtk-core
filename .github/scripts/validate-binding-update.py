import base64
import copy
import json
import re
import stat
import sys
import tomllib
import zipfile
from pathlib import Path


MAX_FILE_BYTES = 4 * 1024 * 1024
MAX_ARCHIVE_BYTES = 8 * 1024 * 1024
FILES = {"Cargo.toml", "Cargo.lock"}
CORE_GIT = "https://github.com/mmtk/mmtk-core.git"
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"


def load_toml(content):
    if not content or len(content) > MAX_FILE_BYTES:
        raise ValueError("Invalid metadata file size")
    return tomllib.loads(content.decode("utf-8"))


def read_archive(archive):
    if archive.stat().st_size > MAX_ARCHIVE_BYTES:
        raise ValueError("Dependency artifact is too large")
    with zipfile.ZipFile(archive) as package:
        entries = package.infolist()
        if len(entries) != 2 or {entry.filename for entry in entries} != FILES:
            raise ValueError("Artifact must contain only Cargo.toml and Cargo.lock")
        contents = {}
        for entry in entries:
            mode = stat.S_IFMT(entry.external_attr >> 16)
            if mode not in (0, stat.S_IFREG) or entry.flag_bits & 1:
                raise ValueError("Artifact entries must be unencrypted regular files")
            if entry.file_size > MAX_FILE_BYTES:
                raise ValueError("Artifact entry is too large")
            # Never extract an archive from the build job into the publishing workspace.
            with package.open(entry) as stream:
                contents[entry.filename] = stream.read(MAX_FILE_BYTES + 1)
            if len(contents[entry.filename]) != entry.file_size:
                raise ValueError("Invalid artifact entry size")
        return contents


def validate(contents, original, core_manifest, core_commit):
    if not re.fullmatch(r"[0-9a-f]{40}", core_commit):
        raise ValueError("Invalid core commit")
    before = load_toml(original["Cargo.toml"])
    after = load_toml(contents["Cargo.toml"])
    expected = copy.deepcopy(before)
    dependency = expected.get("dependencies", {}).get("mmtk")
    if not isinstance(dependency, dict):
        raise ValueError("Expected a table for the mmtk dependency")
    dependency["git"] = CORE_GIT
    dependency["rev"] = core_commit
    if any(key in dependency for key in ("path", "branch", "tag", "registry")):
        raise ValueError("Unexpected alternative mmtk dependency source")
    if json.dumps(after, sort_keys=True) != json.dumps(expected, sort_keys=True):
        raise ValueError("Manifest changes extend beyond the mmtk git/rev update")

    old_lock = load_toml(original["Cargo.lock"])
    lock = load_toml(contents["Cargo.lock"])
    core = load_toml(core_manifest)
    core_versions = {"mmtk": core["package"]["version"]}
    # Path dependencies of a Git package share its pinned Git source.
    for name, dependency in core.get("dependencies", {}).items():
        if isinstance(dependency, dict) and "path" in dependency:
            core_versions[dependency.get("package", name)] = dependency["version"]
    expected_source = f"git+{CORE_GIT}?rev={core_commit}#{core_commit}"
    allowed_sources = {CRATES_IO}
    old_local_packages = set()
    for package in old_lock.get("package", []):
        if "source" in package:
            allowed_sources.add(package["source"].split("#", 1)[0])
        else:
            old_local_packages.add((package["name"], package["version"]))
    if type(lock.get("version")) is not int or lock["version"] not in (3, 4):
        raise ValueError("Unsupported lockfile version")
    if set(lock) - {"version", "package"} or not isinstance(lock.get("package"), list):
        raise ValueError("Invalid lockfile structure")
    seen = set()
    core_packages = set()
    for package in lock["package"]:
        if not isinstance(package, dict) or set(package) - {"name", "version", "source", "checksum", "dependencies"}:
            raise ValueError("Invalid lockfile package")
        name, version, source = package.get("name"), package.get("version"), package.get("source")
        if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_-]+", name) or not isinstance(version, str):
            raise ValueError("Invalid lockfile package identity")
        if source is not None and not isinstance(source, str):
            raise ValueError("Invalid lockfile package source")
        identity = (name, version, source)
        if identity in seen:
            raise ValueError("Duplicate lockfile package")
        seen.add(identity)
        if name in core_versions:
            core_packages.add(name)
            if version != core_versions[name] or source != expected_source:
                raise ValueError("Lockfile does not pin the expected mmtk commit")
        elif source is None:
            if (name, version) not in old_local_packages:
                raise ValueError("Unexpected local lockfile package")
        elif source.startswith("git+"):
            origin, separator, revision = source.rpartition("#")
            if origin not in allowed_sources or not separator or not re.fullmatch(r"[0-9a-f]{40}", revision):
                raise ValueError("Lockfile introduces an unexpected git dependency source")
        elif source not in allowed_sources:
            raise ValueError("Lockfile introduces an unexpected dependency source")
        checksum = package.get("checksum")
        if source is not None and source.startswith("registry+"):
            if not isinstance(checksum, str) or not re.fullmatch(r"[0-9a-f]{64}", checksum):
                raise ValueError("Invalid dependency checksum")
        elif checksum is not None:
            raise ValueError("Unexpected checksum for a non-registry dependency")
        dependencies = package.get("dependencies", [])
        if not isinstance(dependencies, list) or not all(isinstance(item, str) for item in dependencies):
            raise ValueError("Invalid dependency list")
    if core_packages != set(core_versions):
        raise ValueError("Lockfile is missing an expected core package")


def main():
    directory = Path(sys.argv[1])
    expected = json.loads((directory / "expected.json").read_text())
    contents = read_archive(directory / "update.zip")
    original = {
        file: base64.b64decode(expected["original"][f"mmtk/{file}"], validate=True)
        for file in FILES
    }
    validate(contents, original, base64.b64decode(expected["coreManifest"], validate=True), expected["coreCommit"])
    result = {file: base64.b64encode(contents[file]).decode("ascii") for file in FILES}
    with (directory / "validated.json").open("x") as output:
        json.dump(result, output)


if __name__ == "__main__":
    main()