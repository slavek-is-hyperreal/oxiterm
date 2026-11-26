#include <stdint.h>
#include <stdlib.h>
static void merge_sort(int32_t *a, int32_t *tmp, int32_t lo, int32_t hi) {
  if (hi - lo < 2) return;
  int32_t mid = lo + (hi - lo) / 2;
  merge_sort(a, tmp, lo, mid);
  merge_sort(a, tmp, mid, hi);
  int32_t i = lo, j = mid, k = lo;
  while (i < mid && j < hi) {
    if (a[i] <= a[j]) { tmp[k] = a[i]; i++; } else { tmp[k] = a[j]; j++; }
    k++;
  }
  while (i < mid) { tmp[k] = a[i]; i++; k++; }
  while (j < hi) { tmp[k] = a[j]; j++; k++; }
  for (int32_t t = lo; t < hi; t++) a[t] = tmp[t];
}
uint32_t run(int32_t n) {
  uint32_t s = 2463534242u;
  int32_t *a = malloc(sizeof(int32_t) * n), *tmp = malloc(sizeof(int32_t) * n);
  for (int32_t i = 0; i < n; i++) {
    s ^= s << 13; s ^= s >> 17; s ^= s << 5;
    a[i] = (int32_t)(s >> 1);
  }
  merge_sort(a, tmp, 0, n);
  uint32_t h = 0;
  for (int32_t i = 0; i < n; i++) h = h * 31u + (uint32_t)a[i];
  free(a); free(tmp);
  return h;
}
#include "../../drivers/c_harness.h"
