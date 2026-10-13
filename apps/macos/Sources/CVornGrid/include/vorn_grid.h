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
    uint8_t cursor_visible, cursor_style /* 0 block, 1 hollow, 2 bar, 3 underline */;
    uint8_t mouse;  /* 1 when the program tracks the mouse */
    uint8_t screen; /* VG_ALTERNATE | VG_SCROLLED */
    uint32_t fg, bg, cursor_color, nruns;
    const VgRun *runs;
    const uint8_t *text;
    uint32_t text_len;
    void *owner; /* private, freed by vg_view_free */
    uint64_t first_line; /* absolute line of row 0: selections are (line, col) */
} VgView;

enum { VG_ALTERNATE = 1, VG_SCROLLED = 2 };

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
/* A paste: \n becomes \r, bracketed when the program asks for it. */
void vg_paste(VgClient *c, uint32_t pane, const char *utf8);
/* A wheel turn of lines (negative up) at cell (x, y): sent to a program tracking the mouse, as arrow
   keys on the alternate screen, else scrolls the view through history. */
void vg_wheel(VgClient *c, uint32_t pane, int32_t lines, float x, float y, uint16_t mods);
/* Scrolls the view through history (negative up); INT32_MAX returns to the live screen.
   Typing also returns to it. */
void vg_scroll(VgClient *c, uint32_t pane, int32_t lines);
/* For a program tracking the mouse: action 0 press, 1 release, 2 motion; button 0 none, 1 left,
   2 middle, 3 right; x, y in cells. */
void vg_mouse(VgClient *c, uint32_t pane, uint8_t action, uint8_t button, uint16_t mods, float x, float y);
/* Text from (from_line, from_col) to (to_line, to_col), end excluded; free with vg_free_string. */
char *vg_text_range(VgClient *c, uint32_t pane, uint64_t from_line, uint16_t from_col, uint64_t to_line,
                    uint16_t to_col);
/* The columns [from, to) of the word at (line, col). */
bool vg_word_at(VgClient *c, uint32_t pane, uint64_t line, uint16_t col, uint16_t *from, uint16_t *to);
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
