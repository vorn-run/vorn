// The phone app's C API (src/ffi.rs); call it on the main thread only.
#pragma once
#include <stdbool.h>
#include <stdint.h>

typedef struct VpHandle VpHandle;
typedef void (*VpWake)(void *ctx);

VpHandle *vp_new(const char *host, const char *token, const char *pin, VpWake wake, void *ctx);
void vp_free(VpHandle *h);

bool vp_attach_layer(VpHandle *h, void *metal_layer, float w, float h_pt, float scale, bool terminal_only);
void vp_detach_layer(VpHandle *h);
void vp_resize(VpHandle *h, float w, float h_pt, float scale, float top, float bottom, float left, float right, float keyboard);
bool vp_frame(VpHandle *h);
bool vp_pump(VpHandle *h);
uint64_t vp_revision(VpHandle *h);

void vp_touch(VpHandle *h, int32_t phase, float x, float y);
void vp_text(VpHandle *h, const char *s);
void vp_preedit(VpHandle *h, const char *s);
void vp_key(VpHandle *h, const char *code, uint16_t mods);
void vp_send(VpHandle *h, const char *text);
void vp_call(VpHandle *h, const char *method, const char *params);
bool vp_wants_keyboard(VpHandle *h);
void vp_keyboard_hidden(VpHandle *h);
void vp_open_session(VpHandle *h, const char *id);

char *vp_native_tree(VpHandle *h, const char *screen);
void vp_native_action(VpHandle *h, const char *screen, const char *action_json);
char *vp_take_go(VpHandle *h);
char *vp_prefs(VpHandle *h);
char *vp_markdown_key(const char *text, const char *key, uint32_t start, uint32_t end);
void vp_set_prefs(VpHandle *h, const char *json);

char *vp_metrics(VpHandle *h);
void vp_metrics_reset(VpHandle *h);
void vp_free_string(char *s);
uint8_t *vp_svg_rgba(const char *svg, uint32_t px);
void vp_free_rgba(uint8_t *p, uint32_t px);
