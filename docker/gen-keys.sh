#!/bin/sh
# Generates throwaway test keys into docker/keys (gitignored).
set -e
cd "$(dirname "$0")/keys"
[ -f ed25519 ] || ssh-keygen -q -t ed25519 -N '' -f ed25519
[ -f rsa_enc ] || ssh-keygen -q -t rsa -b 3072 -N 'testpass' -f rsa_enc
cat ed25519.pub rsa_enc.pub > authorized_keys
