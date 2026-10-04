#!/bin/sh
# MODE=password | key
set -e
if [ -f /keys/authorized_keys ]; then
  install -o tester -g tester -m 600 /keys/authorized_keys /home/tester/.ssh/authorized_keys
fi
# Docker socket group access (for remote container management tests)
if [ -S /var/run/docker.sock ]; then
  gid=$(stat -c %g /var/run/docker.sock)
  getent group "$gid" >/dev/null || addgroup -g "$gid" dockersock
  adduser tester "$(getent group "$gid" | cut -d: -f1)"
fi
if [ "$MODE" = "key" ]; then
  # alpine locks accounts without password hash for sshd; set a random one
  echo "tester:$(head -c16 /dev/urandom | base64)" | chpasswd
  PA=no
else
  PA=yes
fi
exec /usr/sbin/sshd -D -e \
  -o PasswordAuthentication=$PA -o KbdInteractiveAuthentication=$PA \
  -o PubkeyAuthentication=$([ "$MODE" = key ] && echo yes || echo no)
