#include <ApplicationServices/ApplicationServices.h>
#include <Carbon/Carbon.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void usage(void) {
    fprintf(stderr,
            "usage: post-ax-key-repeat --pid <pid> [--char <char>] [--keycode <code>] [--count <n>] [--interval-ms <ms>] [--initial-delay-ms <ms>]\n");
}

int main(int argc, char **argv) {
    pid_t pid = 0;
    CGCharCode key_char = 'j';
    CGKeyCode key_code = kVK_ANSI_J;
    int count = 40;
    useconds_t interval_us = 33333;
    useconds_t initial_delay_us = 250000;

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--pid") == 0 && i + 1 < argc) {
            pid = (pid_t)atoi(argv[++i]);
        } else if (strcmp(argv[i], "--char") == 0 && i + 1 < argc) {
            key_char = (CGCharCode)argv[++i][0];
        } else if (strcmp(argv[i], "--keycode") == 0 && i + 1 < argc) {
            key_code = (CGKeyCode)atoi(argv[++i]);
        } else if (strcmp(argv[i], "--count") == 0 && i + 1 < argc) {
            count = atoi(argv[++i]);
        } else if (strcmp(argv[i], "--interval-ms") == 0 && i + 1 < argc) {
            interval_us = (useconds_t)(atof(argv[++i]) * 1000.0);
        } else if (strcmp(argv[i], "--initial-delay-ms") == 0 && i + 1 < argc) {
            initial_delay_us = (useconds_t)(atof(argv[++i]) * 1000.0);
        } else {
            usage();
            return 2;
        }
    }

    if (pid == 0) {
        usage();
        return 2;
    }

    AXUIElementRef app = AXUIElementCreateApplication(pid);
    usleep(initial_delay_us);
    for (int i = 0; i < count; i++) {
        AXError down = AXUIElementPostKeyboardEvent(app, key_char, key_code, true);
        AXError up = AXUIElementPostKeyboardEvent(app, key_char, key_code, false);
        if (down != kAXErrorSuccess || up != kAXErrorSuccess) {
            fprintf(stderr, "AX post failed: %d/%d\n", down, up);
            CFRelease(app);
            return 1;
        }
        usleep(interval_us);
    }
    CFRelease(app);
    return 0;
}
