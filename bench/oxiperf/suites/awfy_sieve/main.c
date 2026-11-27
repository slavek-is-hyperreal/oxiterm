#include <stdint.h>
#include <stdlib.h>
#include <string.h>
uint32_t run(int32_t n) {
  uint32_t total = 0;
  for (int32_t rep = 0; rep < n; rep++) {
    uint8_t *flags = malloc(5000);
    memset(flags, 1, 5000);
    int32_t primeCount = 0;
    for (int32_t i = 2; i <= 5000; i++) {
      if (flags[i - 1]) {
        primeCount += 1;
        int32_t k = i + i;
        while (k <= 5000) { flags[k - 1] = 0; k += i; }
      }
    }
    total += (uint32_t)primeCount;
    free(flags);
  }
  return total;
}
#include "../../drivers/c_harness.h"
