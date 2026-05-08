# OxiTerm — Raport Po-Audytowy (Weryfikacja: 2026-05-06)

> **Status:** ✅ Krytyczne błędy (Security & Reliability) naprawione.  
> **Ocena po poprawkach:** 8.5 / 10

---

## ✅ NAPRAWIONE BŁĘDY KRYTYCZNE (Fixed)

### BUG-C01 — Luka auth_password
**Status:** ✅ NAPRAWIONE  
Wdrożono realną weryfikację hasła w `server.rs` przeciwko konfiguracji. Tryb "test user" został usunięty.

### BUG-C02 — Persystencja kluczy hosta
**Status:** ✅ NAPRAWIONE  
`keys.rs` poprawnie zapisuje wygenerowane klucze na dysku (`fs::write`) i ustawia uprawnienia `600`.

### BUG-C03 — Cleanup terminala (Dead Code)
**Status:** ✅ NAPRAWIONE  
Wdrożono `TerminalCleanupGuard` w `session.rs`. Dzięki RAII (trait `Drop`), terminal jest przywracany do stanu początkowego (pokazanie kursora, wyjście z Alt Buffer) przy każdym zakończeniu pętli zdarzeń.

### BUG-C04 — Duplikacja BSU/ESU
**Status:** ✅ NAPRAWIONE  
Usunięto mylące komentarze. Emisja sekwencji Synchronized Update jest teraz scentralizowana w `SyncedEmitter`.

---

## ✅ NAPRAWIONE BŁĘDY WYSOKIEJ/ŚREDNIEJ WAGI

- **BUG-H01 (PredictiveEcho FIFO)**: Wdrożono poprawne potwierdzanie znaków z przodu kolejki.
- **BUG-H02 (Overlay Position)**: Predykcyjne echo używa teraz rect.x/y aktywnego węzła input.
- **BUG-H04 (Diff Engine cur_x)**: Dodano brakujące aktualizacje pozycji kursora po `MoveCursor`.
- **BUG-H05 (Layout Cache)**: `clear_dirty()` jest teraz wywoływane poprawnie po każdym przeliczeniu layoutu.
- **BUG-M07 (Unicode PUA)**: Zakres VTM modifier został poprawiony na `U+D0000` (zgodnie ze specyfikacją VTM).
- **DUPLICATION**: Usunięto nadmiarowy plik `limiter.rs` z renderer-a.

---

## ⚠️ POZOSTAŁE DO NAPRAWIENIA (Odpady po-audytowe)

### QUAL-03 — SixelCodec Placeholder
**Powaga:** 🟠 WYSOKA  
Obecnie `SixelCodec` zwraca statyczny czarny prostokąt. Wymagana pełna implementacja enkodera Sixel dla dynamicznych obrazów.

### QUAL-06 — Brak testów jednostkowych
**Powaga:** 🔴 KRYTYCZNA (Jakość)  
Projekt wymaga wdrożenia testów dla `Renderer::render_node` i `DiffEngine`. Obecne pokrycie to 0%.

### QUAL-08 — Negocjator możliwości terminala
**Powaga:** 🟡 ŚREDNIA  
Mechanizm `DA1/DA2` jest zaimplementowany, ale wymaga testów z większą liczbą emulatorów (XTerm, iTerm2, WezTerm).
