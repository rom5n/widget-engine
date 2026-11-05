/* widget.h: C API of the widget engine (widget.dll). Mirrors crates/engine-ffi/src/lib.rs.
 *
 * All calls must come from the thread that created the engine. Strings are UTF-8.
 * Functions return WE_OK (0) or a negative error code; we_last_error() explains it.
 */
#ifndef WIDGET_ENGINE_H
#define WIDGET_ENGINE_H

#ifdef __cplusplus
extern "C" {
#endif

#define WE_OK 0
#define WE_INVALID_ARGUMENT (-1) /* null pointer or non-UTF-8 string */
#define WE_LOAD_FAILED (-2)      /* invalid widget package: file and line in we_last_error() */
#define WE_RUNTIME_ERROR (-3)    /* Windows or GPU failure */
#define WE_NO_WIDGET (-4)        /* no widget with that id */

typedef struct WeEngine WeEngine;

/* (user, widget id, action) for action="..." the engine does not handle itself. */
typedef void (*WeActionFn)(void *user, const char *widget, const char *action);
/* (user, widget id, x, y): card top-left corner in screen pixels after the user moved it. */
typedef void (*WeMovedFn)(void *user, const char *widget, int x, int y);
/* context is an ID2D1DeviceContext*, valid only during the call. Size in DIPs.
 * Return 0 (S_OK) or an HRESULT. */
typedef int (*WePaintFn)(void *user, void *context, float width, float height);

/* Message of the last failed call on this thread. */
const char *we_last_error(void);

/* Creates an engine. Never NULL. */
WeEngine *we_engine_new(void);
/* Frees an engine that was never passed to we_run / we_start. */
void we_engine_free(WeEngine *engine);

/* Loads a widget package directory (widget.toml, layout.xml, assets).
 * id: stable across runs, keys the saved position and settings; no '.', '=' or newline.
 * x, y: default top-left corner in screen pixels. */
int we_load(WeEngine *engine, const char *id, const char *dir, int x, int y);

/* Sets {app.key} in a package widget. Works before and while the engine runs. */
int we_set(const char *widget, const char *key, const char *value);

int we_on_action(WeEngine *engine, WeActionFn f, void *user);
int we_on_moved(WeEngine *engine, WeMovedFn f, void *user);
/* Paint callback for <canvas id="canvas"/> in a loaded package widget. */
int we_on_paint(WeEngine *engine, const char *widget, const char *canvas, WePaintFn f, void *user);
/* Reloads package widgets when their files change (development). */
int we_hot_reload(WeEngine *engine);

/* Shows the widgets and runs the message loop until all widgets close.
 * Takes ownership of the engine. */
int we_run(WeEngine *engine);
/* Shows the widgets and returns; the app's own message loop drives them.
 * Takes ownership of the engine. Call we_stop() before exiting. */
int we_start(WeEngine *engine);
void we_stop(void);

#ifdef __cplusplus
}
#endif

#endif
