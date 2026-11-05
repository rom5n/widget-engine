/* C app hosting two widget packages through widget.dll: the clock, and the sparkline whose
 * Refresh button calls back into this app, which answers by setting {app.peak}.
 *
 * Build (x64 Native Tools prompt, after `cargo build --release -p widget-engine-ffi`):
 *   cl /nologo /W4 /I ..\..\crates\engine-ffi\include main.c ..\..\target\release\widget.dll.lib
 * Run from this directory with widget.dll next to main.exe or on PATH.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "widget.h"

static void on_action(void *user, const char *widget, const char *action) {
    int *refreshes = (int *)user;
    printf("%s: %s\n", widget, action);
    if (strcmp(action, "refresh") == 0) {
        char peak[16];
        ++*refreshes;
        snprintf(peak, sizeof peak, "%d%%", 50 + (*refreshes * 17) % 50);
        we_set(widget, "peak", peak);
    }
}

static void on_moved(void *user, const char *widget, int x, int y) {
    (void)user;
    printf("%s moved to %d, %d\n", widget, x, y);
}

int main(void) {
    static int refreshes = 0;
    WeEngine *engine = we_engine_new();
    if (we_load(engine, "c-clock", "../widgets/clock", 1200, 60) != WE_OK ||
        we_load(engine, "c-sparkline", "../widgets/sparkline", 1200, 260) != WE_OK) {
        fprintf(stderr, "load failed: %s\n", we_last_error());
        we_engine_free(engine);
        return 1;
    }
    we_set("c-sparkline", "peak", "--");
    we_on_action(engine, on_action, &refreshes);
    we_on_moved(engine, on_moved, NULL);
    if (we_run(engine) != WE_OK) {
        fprintf(stderr, "run failed: %s\n", we_last_error());
        return 1;
    }
    return 0;
}
