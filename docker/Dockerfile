FROM alpine:3.22
RUN apk add --no-cache openssh sudo bash docker-cli shadow rsync vsftpd \
 && ssh-keygen -A \
 && adduser -D -s /bin/bash tester \
 && echo 'tester:hunter2' | chpasswd \
 && echo 'tester ALL=(ALL) ALL' > /etc/sudoers.d/tester \
 && mkdir -p /home/tester/.ssh && chmod 700 /home/tester/.ssh \
 && chown tester:tester /home/tester/.ssh
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
EXPOSE 22
ENTRYPOINT ["/entrypoint.sh"]
