#!/usr/bin/env bash
set -euo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd -- "${script_directory}/../.." && pwd)"
host_binary="${1:-${repository_root}/target/debug/milo-native-host}"
manifest_template="${script_directory}/com.milo.desktop.json"
install_directory="${HOME:?HOME must be set}/.mozilla/native-messaging-hosts"
installed_manifest="${install_directory}/com.milo.desktop.json"

if [[ ! -x "${host_binary}" ]]; then
  echo "Native host binary is missing or not executable: ${host_binary}" >&2
  echo "Build it first with: cargo build --bin milo-native-host" >&2
  exit 1
fi

host_binary="$(realpath -- "${host_binary}")"
escaped_host_binary="${host_binary//&/\\&}"
temporary_manifest="$(mktemp)"
trap 'rm -f -- "${temporary_manifest}"' EXIT

sed "s&__MILO_NATIVE_HOST_PATH__&${escaped_host_binary}&" \
  "${manifest_template}" > "${temporary_manifest}"

install -d -m 700 -- "${install_directory}"
install -m 600 -- "${temporary_manifest}" "${installed_manifest}"

echo "Installed Firefox native host manifest:"
echo "  ${installed_manifest}"
echo "Native host executable:"
echo "  ${host_binary}"
