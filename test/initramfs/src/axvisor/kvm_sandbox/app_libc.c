// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <string.h>

int main(int argument_count, char *arguments[])
{
	if (argument_count != 2 || strcmp(arguments[1], "libc-ok") != 0)
		return 1;
	return puts("sandbox libc application pass!") < 0 ? 1 : 0;
}
