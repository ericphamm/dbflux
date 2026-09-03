#!/bin/bash
#
# Create the self-signed certificate that local builds are signed with.
#
# Run this once. After it, `scripts/dev-install.sh` signs every build with the
# same identity, and the keychain stops asking for permission after each
# rebuild.
#
# Why it is needed:
#
#   macOS ties a keychain grant ("Always Allow") to the signature of the
#   application that asked. An ad-hoc signature has no identity, so the grant
#   is tied to that exact binary — the next build is a different binary and
#   therefore, as far as the keychain is concerned, a different application.
#   Hence the password prompt after every install. A certificate gives the
#   signature a stable identity, the grant matches every later build, and the
#   prompt appears once.
#
# This certificate is local and self-signed. It says nothing about who wrote
# the software: it exists so this Mac recognises its own builds. Gatekeeper
# still treats the app as unidentified.
#
# Usage:
#   scripts/create-signing-cert.sh              # create "DBFlux Dev"
#   DBFLUX_SIGN_IDENTITY="Name" scripts/…       # or under another name
#
# The script asks for your login keychain password. It is typed straight into
# the `security` tool and is neither stored nor echoed.

set -euo pipefail

IDENTITY="${DBFLUX_SIGN_IDENTITY:-DBFlux Dev}"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: this only applies to macOS" >&2
  exit 1
fi

if security find-identity -v -p codesigning | grep -q "\"$IDENTITY\""; then
  echo "'$IDENTITY' already exists — nothing to do."
  echo "Run scripts/dev-install.sh and it will be used."
  exit 0
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

# The system OpenSSL, not whatever is first on PATH. A Homebrew or Anaconda
# OpenSSL 3 writes PKCS#12 files with algorithms Apple's importer rejects, and
# the failure reads as a wrong password rather than an unsupported format.
OPENSSL=/usr/bin/openssl

# The bundle is handed straight to `security import` below and deleted with the
# working directory; it exists for the length of this script. It still needs a
# password, because macOS refuses to import a bundle whose MAC covers an empty
# one — that, not the algorithms, is what makes an empty password fail.
BUNDLE_PASSWORD="$(uuidgen)"

echo "==> creating a self-signed code-signing certificate: $IDENTITY"
# `codeSigning` in the extended key usage is what makes `codesign` accept the
# certificate; without it the identity exists but is never offered.
"$OPENSSL" req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes \
  -keyout "$workdir/key.pem" \
  -out "$workdir/cert.pem" \
  -subj "/CN=$IDENTITY" \
  -addext "basicConstraints=critical,CA:false" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning" \
  >/dev/null 2>&1

"$OPENSSL" pkcs12 -export \
  -inkey "$workdir/key.pem" \
  -in "$workdir/cert.pem" \
  -out "$workdir/identity.p12" \
  -passout "pass:$BUNDLE_PASSWORD" \
  >/dev/null 2>&1

echo "==> importing it into your login keychain"
# -T grants codesign access to the private key without a prompt per signature.
security import "$workdir/identity.p12" -k "$KEYCHAIN" \
  -P "$BUNDLE_PASSWORD" -T /usr/bin/codesign >/dev/null

echo "==> trusting it for code signing"
echo "    (macOS will ask you to confirm)"
security add-trusted-cert -p codeSign -k "$KEYCHAIN" "$workdir/cert.pem"

echo
echo "==> unlocking the key for codesign"
echo "    Enter your login keychain password — the same one you use to log in."
echo "    Without this step every signature prompts for permission, which is"
echo "    the problem this script exists to solve."
read -r -s -p "    Password: " keychain_password
echo

security set-key-partition-list \
  -S apple-tool:,apple:,codesign: \
  -s -k "$keychain_password" "$KEYCHAIN" >/dev/null
unset keychain_password

echo
if security find-identity -v -p codesigning | grep -q "\"$IDENTITY\""; then
  echo "'$IDENTITY' is ready."
  echo
  echo "Next: run scripts/dev-install.sh, launch the app, and answer the"
  echo "keychain prompt with 'Always Allow'. It should be the last one."
else
  echo "error: '$IDENTITY' was created but is not offered for code signing." >&2
  echo "Check Keychain Access: the certificate must be trusted for code signing." >&2
  exit 1
fi
