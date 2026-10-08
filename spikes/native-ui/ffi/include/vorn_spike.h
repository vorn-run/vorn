// C ABI of the spike's grid client (ffi/src/lib.rs).
#pragma once
#include <stdbool.h>
#include <stdint.h>

typedef struct VsHandle VsHandle;

typedef struct {
    uint16_t row, col, ncols, flags;
    uint32_t fg, bg, text_off, text_len;
} VsRun;

enum {
    VS_BOLD = 1, VS_ITALIC = 2, VS_UNDERLINE = 4, VS_STRIKE = 8,
    VS_WIDE = 16, VS_FAINT = 32, VS_CLUSTER = 64,
};

typedef struct {
    uint64_t rev;
    uint16_t cols, rows, cursor_x, cursor_y;
    uint8_t cursor_visible, cursor_style, probe_hit, _pad;
    uint32_t fg, bg, cursor_color, nruns;
    const VsRun *runs;
    const uint8_t *text;
    uint32_t text_len;
    void *owner;
} VsView;

VsHandle *vs_open(uint16_t cols, uint16_t rows);
void vs_set_waker(const VsHandle *h, void (*cb)(void *), void *ctx);
uint32_t vs_panes(const VsHandle *h);
uint32_t vs_take_dirty(const VsHandle *h, uint32_t *out, uint32_t cap);
bool vs_all_snapshotted(const VsHandle *h);
bool vs_closed(const VsHandle *h);
bool vs_view(const VsHandle *h, uint32_t pane, VsView *out);
void vs_view_free(VsView *v);
void vs_key(const VsHandle *h, uint32_t pane, const char *code, uint16_t mods, const char *text);
void vs_text(const VsHandle *h, uint32_t pane, const char *utf8);
void vs_resize(const VsHandle *h, uint32_t pane, uint16_t cols, uint16_t rows);
/* 0: the host draws the pane (option A), 1: the GPU renderer (option B).
   VORN_SPIKE_RENDERER sets it at open: "swift", "gpu" or per pane "abab". */
uint32_t vs_pane_renderer(const VsHandle *h, uint32_t pane);
void vs_set_pane_renderer(const VsHandle *h, uint32_t pane, uint32_t r);
/* The pane's screen text for accessibility; free with vs_free_text. */
char *vs_read_text(const VsHandle *h, uint32_t pane);
void vs_free_text(char *t);

uint32_t vs_bench_mode(const VsHandle *h);
uint32_t vs_bench_tick(const VsHandle *h, uint32_t *ch);
void vs_bench_frame(const VsHandle *h, double at_ms, double work_ms);
void vs_bench_first_frame(const VsHandle *h);
void vs_bench_latency(const VsHandle *h, double ms);
void vs_bench_set_period(const VsHandle *h, double ms);
/* Takes the look test's screenshot of this process's window, if asked for. */
void vs_bench_shoot(const VsHandle *h);
void vs_bench_write(const VsHandle *h);
void vs_bench_write_as(const VsHandle *h, const char *client);
/* The probe key was typed now / a probe-hit view is now on screen. */
void vs_bench_typed(const VsHandle *h);
void vs_bench_hit(const VsHandle *h);
/* The host drew `pane` with a screen in it (first-frame bookkeeping). */
void vs_pane_shown(const VsHandle *h, uint32_t pane);
double vs_now_ms(const VsHandle *h);
