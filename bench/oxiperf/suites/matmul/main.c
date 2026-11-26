#include <stdint.h>
#include <stdlib.h>
uint32_t run(int32_t n) {
  double *a = malloc(sizeof(double) * n * n), *b = malloc(sizeof(double) * n * n), *c = malloc(sizeof(double) * n * n);
  int32_t p = 0;
  for (int32_t i = 0; i < n; i++) for (int32_t j = 0; j < n; j++) {
    a[p] = (double)((i * 31 + j * 17) % 13) / 4.0;
    b[p] = (double)((i * 7 + j * 29) % 11) / 4.0;
    p++;
  }
  p = 0;
  for (int32_t i = 0; i < n; i++) for (int32_t j = 0; j < n; j++) {
    double acc = 0.0;
    for (int32_t k = 0; k < n; k++) acc = acc + a[i * n + k] * b[k * n + j];
    c[p++] = acc;
  }
  double total = 0.0;
  for (int32_t i = 0; i < n * n; i++) total = total + c[i];
  free(a); free(b); free(c);
  return (uint32_t)(total * 16.0);
}
#include "../../drivers/c_harness.h"
