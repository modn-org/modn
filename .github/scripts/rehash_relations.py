"""Type and value checks for dependency versions and conflicts.

The rules mirror `PackageManifest::validate` in modn-core, so a converted
record cannot list a relation that Modn would refuse to load.
"""

from __future__ import annotations

from typing import Any

MAX_CONFLICTS = 32


class RehashError(Exception):
    """A package or bucket problem that stops the rehash without writes."""


def valid_id(value: Any) -> bool:
    """Return true for a bucket identifier, mirroring the v2 path rule."""
    lowercase = "abcdefghijklmnopqrstuvwxyz"
    return (
        isinstance(value, str)
        and 0 < len(value) <= 64
        and value[0] in lowercase
        and all(
            character in lowercase or character in "0123456789" or character == "-"
            for character in value
        )
        and not value.endswith("-")
    )


def valid_numeric_version(value: str) -> bool:
    """Return true for a dotted numeric version, as Modn parses requirements."""
    digits = value[1:] if value.startswith("v") else value
    components = digits.split(".")
    return (
        0 < len(components) <= 8
        and all(
            component
            and all("0" <= character <= "9" for character in component)
            and int(component) <= 2**64 - 1
            for component in components
        )
    )


def valid_version_requirement(value: Any) -> bool:
    """Return true for the comma-separated comparator list Modn accepts."""
    if not isinstance(value, str) or len(value) > 64:
        return False
    parts = [part.strip() for part in value.split(",")]
    if len(parts) > 4:
        return False
    for part in parts:
        for operator in (">=", "<=", ">", "<", "="):
            if part.startswith(operator):
                version = part[len(operator):].strip()
                break
        else:
            return False
        if not valid_numeric_version(version):
            return False
    return True


def check_relation_fields(filename: str, value: dict[str, Any]) -> None:
    """Refuse dependency versions or conflicts that Modn would reject."""
    dependencies = value.get("dependencies", [])
    if "dependency_versions" in value:
        dependency_versions = value["dependency_versions"]
        if not isinstance(dependency_versions, dict):
            raise RehashError(
                f"{filename}: field dependency_versions must be an object of version requirements"
            )
        for dependency, requirement in dependency_versions.items():
            if not valid_id(dependency):
                raise RehashError(
                    f"{filename}: field dependency_versions names an invalid dependency: {dependency!r}"
                )
            if not isinstance(requirement, str):
                raise RehashError(
                    f"{filename}: field dependency_versions value for {dependency} must be a string"
                )
            if not valid_version_requirement(requirement):
                raise RehashError(
                    f"{filename}: field dependency_versions value for {dependency} is not a valid version requirement"
                )
            if dependency not in dependencies:
                raise RehashError(
                    f"{filename}: field dependency_versions names {dependency}, which is not a dependency"
                )
    if "conflicts" in value:
        conflicts = value["conflicts"]
        if not isinstance(conflicts, list) or not all(
            isinstance(item, str) for item in conflicts
        ):
            raise RehashError(f"{filename}: field conflicts must be a list of strings")
        if len(conflicts) > MAX_CONFLICTS:
            raise RehashError(
                f"{filename}: field conflicts must hold at most {MAX_CONFLICTS} entries"
            )
        package_id = value.get("id")
        seen: set[str] = set()
        for conflict in conflicts:
            if not valid_id(conflict):
                raise RehashError(
                    f"{filename}: field conflicts names an invalid package id: {conflict!r}"
                )
            if conflict == package_id:
                raise RehashError(
                    f"{filename}: field conflicts must not name its own package"
                )
            if conflict in dependencies:
                raise RehashError(
                    f"{filename}: field conflicts must not name dependency {conflict}"
                )
            if conflict in seen:
                raise RehashError(
                    f"{filename}: field conflicts must not repeat {conflict}"
                )
            seen.add(conflict)
