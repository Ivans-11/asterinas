// SPDX-License-Identifier: MPL-2.0

#include <arpa/inet.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static int fail(const char *operation)
{
	fprintf(stderr, "%s failed: %s\n", operation, strerror(errno));
	return 1;
}

static int test_tcp(void)
{
	static const char request[] = "gvisor-tcp-request";
	static const char response[] = "gvisor-tcp-response";
	struct sockaddr_in address = {
		.sin_family = AF_INET,
		.sin_port = htons(18080),
		.sin_addr.s_addr = htonl(INADDR_LOOPBACK),
	};
	char buffer[64] = { 0 };
	int listener = -1;
	int client = -1;
	int server = -1;
	int result = 1;

	listener = socket(AF_INET, SOCK_STREAM, 0);
	if (listener < 0) {
		return fail("TCP socket");
	}
	if (bind(listener, (struct sockaddr *)&address, sizeof(address)) < 0) {
		fail("TCP bind");
		goto out;
	}
	if (listen(listener, 1) < 0) {
		fail("TCP listen");
		goto out;
	}

	client = socket(AF_INET, SOCK_STREAM, 0);
	if (client < 0) {
		fail("TCP client socket");
		goto out;
	}
	if (connect(client, (struct sockaddr *)&address, sizeof(address)) < 0) {
		fail("TCP connect");
		goto out;
	}
	server = accept(listener, NULL, NULL);
	if (server < 0) {
		fail("TCP accept");
		goto out;
	}

	if (send(client, request, sizeof(request), 0) != sizeof(request) ||
	    recv(server, buffer, sizeof(buffer), 0) != sizeof(request) ||
	    memcmp(buffer, request, sizeof(request)) != 0) {
		fprintf(stderr, "TCP request exchange failed\n");
		goto out;
	}
	memset(buffer, 0, sizeof(buffer));
	if (send(server, response, sizeof(response), 0) != sizeof(response) ||
	    recv(client, buffer, sizeof(buffer), 0) != sizeof(response) ||
	    memcmp(buffer, response, sizeof(response)) != 0) {
		fprintf(stderr, "TCP response exchange failed\n");
		goto out;
	}

	result = 0;
out:
	if (server >= 0)
		close(server);
	if (client >= 0)
		close(client);
	close(listener);
	return result;
}

static int test_udp(void)
{
	static const char message[] = "gvisor-udp-pass";
	struct sockaddr_in address = {
		.sin_family = AF_INET,
		.sin_port = htons(19000),
		.sin_addr.s_addr = htonl(INADDR_LOOPBACK),
	};
	char buffer[64] = { 0 };
	int receiver = -1;
	int sender = -1;
	int result = 1;

	receiver = socket(AF_INET, SOCK_DGRAM, 0);
	if (receiver < 0) {
		return fail("UDP receiver socket");
	}
	if (bind(receiver, (struct sockaddr *)&address, sizeof(address)) < 0) {
		fail("UDP bind");
		goto out;
	}
	sender = socket(AF_INET, SOCK_DGRAM, 0);
	if (sender < 0) {
		fail("UDP sender socket");
		goto out;
	}
	if (sendto(sender, message, sizeof(message), 0,
		   (struct sockaddr *)&address, sizeof(address)) != sizeof(message)) {
		fail("UDP sendto");
		goto out;
	}
	if (recvfrom(receiver, buffer, sizeof(buffer), 0, NULL, NULL) !=
		    sizeof(message) ||
	    memcmp(buffer, message, sizeof(message)) != 0) {
		fprintf(stderr, "UDP exchange failed\n");
		goto out;
	}

	result = 0;
out:
	if (sender >= 0)
		close(sender);
	close(receiver);
	return result;
}

int main(void)
{
	if (test_tcp() != 0 || test_udp() != 0)
		return 1;
	puts("gvisor kvm loopback network test pass!");
	return 0;
}
