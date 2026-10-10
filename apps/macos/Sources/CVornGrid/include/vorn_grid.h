/* C ABI of libvorn_grid_ffi (apps/macos/grid-ffi): live terminals from
   vornd's grid mode. All functions are safe to call from any thread; null
   handles and strings that are not UTF-8 are no-ops. */
#pragma once
#include <stdbool.h>
#include <stdint.h>

typedef struct VgClient VgClient;

typedef struct {
    uint16_t row, col, ncols, flags;
    uint32_t fg, bg, text_off, text_len;
} VgRun;

enum {
    VG_BOLD = 1, VG_ITALIC = 2, VG_UNDERLINE = 4, VG_STRIKE = 8,
    VG_WIDE = 16, VG_FAINT = 32, VG_CLUSTER = 64,
};

typedef struct {
    uint64_t rev;
    uint16_t cols, rows, cursor_x, cursor_y;
    uint8_t cursor_visible, cursor_style /* 0 block, 1 hollow, 2 bar, 3 underline */, _pad0, _pad1;
    uint32_t fg, bg, cursor_color, nruns;
    const VgRun *runs;
    const uint8_t *text;
    uint32_t text_len;
    void *owner; /* private, freed by vg_view_free */
} VgView;

typedef struct {
    uint32_t fg, bg, cursor;
    uint32_t ansi[16];
} VgTheme; /* 0xRRGGBB */

/* Connects to the grid socket and says Hello. NULL on failure. theme may be NULL (built-in defaults). */
VgClient *vg_connect(const char *socket_path, const char *build, const VgTheme *theme);
/* Closes the socket, joins the reader thread and frees. The waker must not block on the calling thread. */
void vg_close(VgClient *c);
/* Called from the reader thread (coalesced: at most once until vg_take_dirty is called) when panes
   changed or the connection state changed. Called once right away if changes are already waiting.
   cb NULL clears it. */
void vg_set_waker(VgClient *c, void (*cb)(void *ctx), void *ctx);
/* Attaches a session in grid mode at cols x rows; returns a local pane id (>0, never reused), 0 on error. */
uint32_t vg_attach(VgClient *c, const char *session_id, uint16_t cols, uint16_t rows);
/* Sends Detach if attached, forgets the pane. */
void vg_detach(VgClient *c, uint32_t pane);
/* Pane ids that changed since the last call. Returns the count written; call again while it equals cap. */
uint32_t vg_take_dirty(VgClient *c, uint32_t *out, uint32_t cap);
/* 0 attaching, 1 live (has a screen), 2 failed (e.g. no such session), 3 connection closed, -1 no such pane */
int32_t vg_pane_state(VgClient *c, uint32_t pane);
bool vg_closed(VgClient *c);
/* false if no screen yet; free a filled view with vg_view_free. */
bool vg_view(VgClient *c, uint32_t pane, VgView *out);
void vg_view_free(VgView *v);
/* code: W3C KeyboardEvent.code ("KeyA", "Enter", "ArrowUp", ...); mods bits: 1 shift, 2 alt, 4 ctrl, 8 meta;
   text may be NULL. */
void vg_key(VgClient *c, uint32_t pane, const char *code, uint16_t mods, const char *text);
/* Committed text or a paste. */
void vg_text(VgClient *c, uint32_t pane, const char *utf8);
/* The pane's viewport changed (what fits): sends Viewport only. */
void vg_viewport(VgClient *c, uint32_t pane, uint16_t cols, uint16_t rows);
/* "Fit to this pane": sends TakeSize. */
void vg_take_size(VgClient *c, uint32_t pane);
/* 0 active, 1 watching, 2 away */
void vg_presence(VgClient *c, uint32_t pane, uint8_t state);
/* Viewport text, rows joined by \n, trailing spaces trimmed; free with vg_free_string. NULL if no screen. */
char *vg_read_text(VgClient *c, uint32_t pane);
/* Last error message (connection or pane errors), or NULL; free with vg_free_string. */
char *vg_last_error(VgClient *c);
void vg_free_string(char *s);
