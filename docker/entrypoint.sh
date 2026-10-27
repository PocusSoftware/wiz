#!/bin/sh
# MODE=password | key | ftp
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
if [ "$MODE" = "ftp" ]; then
  # A server with FTP instead of SFTP: no sftp subsystem, vsftpd on port 21.
  sed -i '/^Subsystem/d' /etc/ssh/sshd_config
  vsftpd -obackground=YES -oseccomp_sandbox=NO /etc/vsftpd/vsftpd.conf
fi
if [ "$MODE" = "key" ]; then
  # alpine locks accounts without password hash for sshd; set a random one
  echo "tester:$(head -c16 /dev/urandom | base64)" | chpasswd
  PA=no
else
  PA=yes
fi
# Forwarding on, so the password server can be a jump host to the others.
exec /usr/sbin/sshd -D -e -o AllowTcpForwarding=yes \
  -o PasswordAuthentication=$PA -o KbdInteractiveAuthentication=$PA \
  -o PubkeyAuthentication=$([ "$MODE" = key ] && echo yes || echo no)
