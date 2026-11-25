# HANDOFF 8.T BAD ACTOR fixes (update after every fix)
Goal: tests/bad_actor_8t.rs + bad_actor_8t_catalogue.rs all green by fixing code.
## Done
(none yet)
## Notes
- F01 (stmt.rs E0114 for never), F03/F15/F16 (expr.rs place/check_place_mutable): DONE
- F09 shifts DONE (binary_result l==r + check_shift_amount E0102)
- F10 F11 F19 DONE (decl.rs HashSets, enum dup E0123, bool dup case). Remaining quadratic-ish scans in decl.rs fields/methods/union discr (897,919,1015,1148) not touched
- F13 (default_depth, E0122), F14a/b (statics need init, E0123) DONE; whole suite otherwise green. TODO order: F20, D1/D2, F04, F17, F18, F08, F07/F12, F02/F14c/d init order, F05/F06 laws, low items, docs, verify
