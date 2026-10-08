// The GPU pane renderer's C API (ffi/src/render.rs): an app handle over an
// open grid client (vorn_spike.h) and one surface per pane, attached to a
// host NSView or UIView. All calls but the callbacks are main-thread only.
#pragma once
#include <stdbool.h>
#include <stdint.h>
#include "vorn_spike.h"

typedef struct VtApp VtApp;
typedef struct VtSurface VtSurface;

enum { VT_CONTENT_CHANGED = 1, VT_FIRST_FRAME = 2 };

/* wakeup(ud): any thread, actions are queued: call vt_app_tick on main.
   action(ud, pane, tag): from vt_app_tick. */
typedef void (*vt_wakeup_cb)(void *ud);
typedef void (*vt_action_cb)(void *ud, uint32_t pane, uint32_t tag);

typedef struct {
    double cell_w, cell_h, pad_x, pad_y;
    uint16_t cursor_x, cursor_y, cols, rows;
} VtMetrics;

VtApp *vt_app_new(const VsHandle *h, vt_wakeup_cb wakeup, vt_action_cb action, void *ud);
void vt_app_tick(const VtApp *app);
void vt_app_free(VtApp *app);

VtSurface *vt_surface_new(const VtApp *app, uint32_t pane, void *view, double scale, double font_pt);
void vt_surface_free(VtSurface *s);
void vt_surface_set_size(VtSurface *s, double w, double h);
void vt_surface_set_scale(const VtSurface *s, double scale);
void vt_surface_set_focus(const VtSurface *s, bool focused);
void vt_surface_key(const VtSurface *s, const char *code, uint16_t mods, const char *text);
void vt_surface_text(const VtSurface *s, const char *utf8);
void vt_surface_preedit(const VtSurface *s, const char *utf8);
void vt_surface_mouse(const VtSurface *s, double x, double y, int32_t button, int32_t action);
void vt_surface_metrics(const VtSurface *s, VtMetrics *out);
/* Free with vs_free_text. */
char *vt_surface_read_text(const VtSurface *s);
