#include <stdint.h>
#include <stdlib.h>
typedef struct Node { struct Node *left, *right; } Node;
static Node *make(int32_t d) {
  Node *n = malloc(sizeof(Node));
  if (d == 0) { n->left = NULL; n->right = NULL; }
  else { n->left = make(d - 1); n->right = make(d - 1); }
  return n;
}
static int32_t count(Node *n) {
  int32_t c = 1;
  if (n->left) c += count(n->left);
  if (n->right) c += count(n->right);
  return c;
}
static void drop(Node *n) {
  if (n->left) drop(n->left);
  if (n->right) drop(n->right);
  free(n);
}
uint32_t run(int32_t n) {
  uint32_t h = 0;
  for (int32_t rep = 0; rep < 8; rep++) {
    Node *t = make(n);
    h = h * 31u + (uint32_t)(count(t) + rep);
    drop(t);
  }
  return h;
}
#include "../../drivers/c_harness.h"
