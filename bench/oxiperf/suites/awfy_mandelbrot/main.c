#include <stdint.h>
uint32_t run(int32_t n) {
  int32_t size = n, sum = 0, byteAcc = 0, bitNum = 0;
  for (int32_t y = 0; y < size; y++) {
    double ci = ((2.0 * y) / size) - 1.0;
    for (int32_t x = 0; x < size; x++) {
      double zrzr = 0.0, zi = 0.0, zizi = 0.0;
      double cr = ((2.0 * x) / size) - 1.5;
      int32_t z = 0, notDone = 1, escape = 0;
      while (notDone && z < 50) {
        double zr = zrzr - zizi + cr;
        zi = 2.0 * zr * zi + ci;
        zrzr = zr * zr;
        zizi = zi * zi;
        if (zrzr + zizi > 4.0) { notDone = 0; escape = 1; }
        z += 1;
      }
      byteAcc = (byteAcc << 1) + escape;
      bitNum += 1;
      if (bitNum == 8) { sum ^= byteAcc; byteAcc = 0; bitNum = 0; }
      else if (x == size - 1) { byteAcc <<= (8 - bitNum); sum ^= byteAcc; byteAcc = 0; bitNum = 0; }
    }
  }
  return (uint32_t)sum;
}
#include "../../drivers/c_harness.h"
