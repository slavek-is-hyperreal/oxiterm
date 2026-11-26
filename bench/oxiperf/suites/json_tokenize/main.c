#include <stdint.h>
#include <stdlib.h>
#include <string.h>
typedef struct { uint8_t *p; size_t len, cap; } Buf;
static void push(Buf *b, uint8_t c) {
  if (b->len == b->cap) { b->cap *= 2; b->p = realloc(b->p, b->cap); }
  b->p[b->len++] = c;
}
static void push_text(Buf *b, const char *s) { for (; *s; s++) push(b, (uint8_t)*s); }
static void push_num(Buf *b, int32_t v) {
  if (v >= 10) push_num(b, v / 10);
  push(b, (uint8_t)(48 + v % 10));
}
uint32_t run(int32_t n) {
  Buf b = { malloc(1024), 0, 1024 };
  push(&b, 91);
  for (int32_t i = 0; i < n; i++) {
    if (i > 0) push(&b, 44);
    push_text(&b, "{\"id\":"); push_num(&b, i);
    push_text(&b, ",\"name\":\"item"); push_num(&b, i % 97);
    push_text(&b, "\",\"ok\":true,\"none\":null,\"v\":[1,22,"); push_num(&b, i * 3);
    push_text(&b, "]}");
  }
  push(&b, 93);
  uint32_t h = 0, tokens = 0;
  size_t i = 0, len = b.len;
  const uint8_t *buf = b.p;
  while (i < len) {
    uint8_t c = buf[i];
    if (c == 34) {
      i++; size_t start = i;
      while (buf[i] != 34) i++;
      h = h * 31u + (uint32_t)(i - start); i++; tokens++;
    } else if (c >= 48 && c <= 57) {
      uint32_t v = 0;
      while (i < len && buf[i] >= 48 && buf[i] <= 57) { v = v * 10u + (uint32_t)(buf[i] - 48); i++; }
      h = h * 31u + v; tokens++;
    } else if (c == 116) { i += 4; h = h * 31u + 1u; tokens++; }
    else if (c == 110) { i += 4; h = h * 31u + 2u; tokens++; }
    else {
      if (c == 123 || c == 125 || c == 91 || c == 93 || c == 44 || c == 58) { h = h * 31u + c; tokens++; }
      i++;
    }
  }
  free(b.p);
  return h + tokens;
}
#include "../../drivers/c_harness.h"
