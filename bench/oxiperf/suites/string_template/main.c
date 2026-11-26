#include <stdint.h>
#include <stdio.h>
#include <string.h>
uint32_t run(int32_t n) {
  static const char *labels[4] = {"alpha", "beta", "gamma", "delta"};
  uint32_t h = 0, total = 0;
  char buf[128];
  for (int32_t i = 0; i < n; i++) {
    const char *label = labels[i % 4];
    int32_t x = i * 3;
    int32_t y = (i * 7) % 1000;
    int32_t d = x - y;
    int len = snprintf(buf, sizeof buf, "row #%d: %s (%d, %d) delta=%d", i, label, x, y, d);
    for (int k = 0; k < len; k++) h = h * 31u + (uint8_t)buf[k];
    total += (uint32_t)len;
  }
  return h + total;
}
#include "../../drivers/c_harness.h"
