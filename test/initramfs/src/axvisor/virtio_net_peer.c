// SPDX-License-Identifier: MPL-2.0

#include <arpa/inet.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

#define PEER_PORT 9001
#define GUEST_PORT 9000
static const char request[] = "virtio-udp-outbound-pass\n";
static const char response[] = "virtio-udp-inbound-pass\n";

int main(void)
{
	struct sockaddr_in peer = {
		.sin_family = AF_INET,
		.sin_port = htons(PEER_PORT),
	};
	struct sockaddr_in guest;
	socklen_t guest_len = sizeof(guest);
	char buffer[64];
	ssize_t received;
	ssize_t sent;
	int socket_fd;

	if (inet_pton(AF_INET, "172.16.0.1", &peer.sin_addr) != 1) {
		perror("inet_pton");
		return 1;
	}

	socket_fd = socket(AF_INET, SOCK_DGRAM, 0);
	if (socket_fd < 0) {
		perror("socket");
		return 1;
	}
	if (bind(socket_fd, (struct sockaddr *)&peer, sizeof(peer)) < 0) {
		perror("bind");
		close(socket_fd);
		return 1;
	}
	received = recvfrom(socket_fd, buffer, sizeof(buffer), 0,
			    (struct sockaddr *)&guest, &guest_len);
	if (received != (ssize_t)(sizeof(request) - 1) ||
	    memcmp(buffer, request, sizeof(request) - 1) != 0) {
		fprintf(stderr, "unexpected UDP request\n");
		close(socket_fd);
		return 1;
	}
	guest.sin_port = htons(GUEST_PORT);
	for (int retry = 0; retry < 10; retry++) {
		sent = sendto(socket_fd, response, sizeof(response) - 1, 0,
			      (struct sockaddr *)&guest, guest_len);
		if (sent != (ssize_t)(sizeof(response) - 1)) {
			perror("sendto");
			close(socket_fd);
			return 1;
		}
		usleep(100000);
	}
	close(socket_fd);
	return 0;
}
