# OxiScript MVP grammar

Status: contract of slice 8.C (2026-11-24). Implemented by slice 8.P (lexer, parser, formatter). The AST it produces is `oxiscript/oxiscript-syntax/src/ast.rs`; the frozen word lists are `oxiscript-syntax/src/keywords.rs`. Source files use the extension `.oxs`.

**Design rule (owner decision 2):** a TypeScript developer must be able to *read* an OxiScript file without learning anything. Same keywords, braces, `import`/`export`, `let`/`const`, arrow functions, template strings, type annotations after `:`. Where soundness, determinism or an unambiguous grammar forces a difference, the spelling stays familiar and the difference is listed in section 6 with a side-by-side example and a reason. Real TypeScript is never parsed: nothing is accepted and then rejected later.

Notation: EBNF with `"x"` for terminals, `{ x }` repetition, `[ x ]` optional, `( a | b )` grouping, `(* comment *)`. Nonterminals are `CamelCase`, lexical tokens are `UPPER`.

---

## 1. Lexical structure

* **Source text** is UTF-8. A leading byte-order mark is ignored. Line terminators: `\n`, `\r\n`.
* **Whitespace** separates tokens and is otherwise insignificant, with exactly one exception: the `?` rule in section 1.5.
* **Comments:** `// to end of line`, `/* block, not nested */`, and `/// doc comment` (attached to the following item or member; text kept in the AST). Comments are trivia and live outside the AST (the formatter keeps them).
* **Identifiers:** `IDENT = [A-Za-z_][A-Za-z0-9_]*` (ASCII in the MVP: no homoglyph attacks, simple tooling; Unicode identifiers are reserved for a later edition). Strings and comments are full UTF-8.
* **Keywords** (never identifiers): `as break case class const continue default do else enum export extends false for function if implements import interface let new null private readonly return static struct switch this true while`.
* **Contextual keywords** (keywords in one position, identifiers elsewhere): `constructor from law mutating of type when`.
* **Reserved words** (frozen now so that adding a feature never breaks a program; using one is `OXS-E0900`): future features `arena async await dyn ensures impl inout invariant isolate kernel match move parallel region requires spawn trait trusted unsafe weak where yield`; TypeScript words that do not exist here `any debugger declare delete instanceof namespace protected public super typeof undefined unknown var with`.
* **Builtin type names** (ordinary identifiers, resolved by the checker): `bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 string void never Array Result`.

### 1.1 Numeric literals

```
INT     = ( DEC | "0x" HEX | "0b" BIN | "0o" OCT ) [ NUMSUFFIX ] ;
DEC     = DIGIT { [ "_" ] DIGIT } ;                 (* separators between digits only *)
FLOAT   = ( DEC "." DEC | "." DEC | DEC EXP | DEC "." DEC EXP | "." DEC EXP ) [ FLOATSUFFIX ] ;
EXP     = ( "e" | "E" ) [ "+" | "-" ] DEC ;
NUMSUFFIX   = "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" ;   (* integer suffixes: INT tokens only *)
FLOATSUFFIX = "f32" | "f64" ;
```

`.5` is a literal, as in TypeScript (a `.` immediately followed by a digit always starts a number; so `a ?.5 : 1` is `a ? .5 : 1`); `5.` is not a literal (a digit must follow the point). Tuple/field access by number is therefore never `t.0` (reserved: `t[0]`).

**INT and FLOAT are disjoint.** A float suffix is allowed only on a FLOAT token (`1.0f32`, `1e3f64`, `.5f32`); `1f32` is an error with the fix `1.0f32`. Integer suffixes attach to INT tokens, including hex/octal/binary (`0xffu8`, `0b1010u8`); a hex literal never takes a float suffix, so `0x1f32` is simply the hex number `0x1f32` (no suffix) and there is no ambiguity. A leading `-` is the unary operator, not part of the literal (so `-128i8` is `-(128i8)`; the checker special-cases the minimum value).

**Typing rules** (the "explicit numeric types" decision made light to write):
1. A suffix fixes the type: `42u8`, `1.5f32`, `7i64`. `1u8` is a `u8` constant, range-checked at compile time (`OXS-E0102`).
2. An unsuffixed integer literal takes the type its context demands if the value fits (`let x: u8 = 200`, `f(3)` where the parameter is `i64`, `a + 1` where `a: u16`). If the context demands a float type, an integer literal is accepted when it is exactly representable (`let x: f64 = 1`). With no context the default is **`i32`** (and `OXS-E0102` if it does not fit).
3. An unsuffixed float literal takes `f32` or `f64` from context, default **`f64`**. A float literal never adapts to an integer type.
4. There is no `number`. Arithmetic never mixes numeric types implicitly: `a + b` needs equal types; use `as`.

### 1.2 Strings and template literals

```
STRING   = '"' { CHAR | ESCAPE } '"' | "'" { CHAR | ESCAPE } "'" ;   (* no raw newline *)
ESCAPE   = "\" ( "n" | "r" | "t" | "0" | "\" | '"' | "'" | "`" | "$" | "x" HEX HEX | "u{" HEX{1,6} "}" ) ;
TEMPLATE = "`" { CHAR | ESCAPE | "${" Expr "}" } "`" ;               (* newlines allowed *)
```

`\xHH` must be ASCII (`<= 0x7F`); `\u{...}` must be a Unicode scalar value (no surrogates). A `string` is UTF-8, immutable, with byte length; there is no O(1) code-point indexing (library: `bytes()`, `chars()`). A template literal is sugar for string concatenation with `toString`-style formatting of each interpolation (numbers, bool, string; other types need an explicit conversion).

### 1.3 Operators and punctuation

```
{ } ( ) [ ] ; , . : ? ?. ?? @ @@ => 
+ - * / %   +% -% *%   +| -| *|   & | ^ ~ << >>   ! && ||
=== !==   < <= > >=   = += -= *= /= %= +%= -%= *%= +|= -|= *|= &= |= ^= <<= >>=   ++ --
```

`+% -% *%` wrap, `+| -| *|` saturate (Zig spelling); plain `+ - *` trap on overflow. `==` and `!=` are *not* tokens of the language: writing them is `OXS-E0005` with the fix "use `===`". `>>>` does not exist (`>>` is arithmetic on signed and logical on unsigned types).

### 1.4 Semicolons

Semicolons are **mandatory** statement terminators (no automatic semicolon insertion). A statement that ends with a block (`if`, `while`, `for`, `switch`, function/struct declarations) needs none. Missing `;` is `OXS-E0004` with an insert-here fix. The formatter inserts them.

### 1.5 The one whitespace-sensitive rule: `?`

`?` has three jobs: ternary `a ? b : c`, the try operator `f()?`, and optional chaining `a?.b`.
* `?.` followed by an identifier, `[` or `(` is **always** optional chaining, whatever precedes it: `foo()?.bar`, `foo() ?.bar`, and a continuation after a line break (`foo()` newline `  ?.bar`) all mean optional chaining (a line break before `?.` does not end the expression). `?.` followed by a digit is `?` then the literal `.5` (`a ?.5 : 1` is a ternary).
* Otherwise a lone `?` **preceded by whitespace or a line break** is the ternary operator.
* A lone `?` **not preceded by whitespace** (`f()?`, `x?;`) is the try operator.

`f() ? a : b`, `f()?` and `a ?.b` are therefore unambiguous for the lexer, and this is the **only** whitespace-sensitive rule in the language (casts, operators and literals never depend on spacing). The formatter canonicalises (`a ? b : c`, `f()?`). A `?` after whitespace with no matching `:` is `OXS-E0006`. (Precedent: Swift's whitespace rules for operators.) *Owner: see decision 1 in section 7.*

Consequence: `f()?.x` is optional chaining on the value of `f()`. To propagate the error first and then access, write `(f()?).x`; applying `?.` to a `Result` is `OXS-E0106` with that fix.

---

## 2. Modules and declarations

```
Module     = { InnerAttr ";" } { Item } EOF ;
InnerAttr  = "@@" Ident [ AttrArgs ] ;                       (* applies to the whole module *)
Item       = { DocComment } { Attribute } [ "export" ] ItemBody ;
ItemBody   = ImportDecl | VarDecl ";" | FnDecl | RecordDecl | InterfaceDecl
           | EnumDecl | TypeAlias | LawDecl ;
```

`export` is not allowed on `import`. There is no `export default`, no re-export, no `declare`, no namespaces and no declaration merging (modules only).

### 2.1 Imports

```
ImportDecl   = "import" ImportClause "from" STRING ";" ;
ImportClause = "{" ImportSpec { "," ImportSpec } [ "," ] "}" | "*" "as" Ident ;
ImportSpec   = Ident [ "as" Ident ] ;
```

The source string is one of: a relative path (`"./geometry"`, resolved against the importing file, extension `.oxs` implied), a package name (`"std/math"`, resolved through the manifest), or a **host module** `"host:name"` (a capability interface declared by the host IDL, see `abi.md`; for example `import { Log } from "host:log";` brings the *type* `Log`, never an ambient value). A fourth form, `"rust:<crate>"`, is reserved for **Rust libraries linked into the program** (section 2.7). Cyclic imports are an error in the MVP.

**Entry point and capabilities.** A program receives its capabilities as parameters of an exported function named **`main`**: every parameter of `main` has a host-interface type (an imported `host:` type such as `Log`), no other parameter kinds are allowed, and the return type is `void`. The compiler writes one `needs` line per `main` parameter into the manifest, in parameter order; the host passes the matching capability handles, in that order, to `oxi_init` (see `abi.md` 3.1), and `oxi_init` runs the module-level initialisers and then calls `main`. A module without `main` simply needs no capabilities and exports plain functions. Capability values cannot be created inside the program, can be stored in variables, struct fields and closures, and can be passed to functions; code that is not handed one cannot use it (no ambient authority).
```ts
import { Log } from "host:log";
import { State } from "host:state";
export function main(log: Log, state: State): void {
  log.write(2, "started");
}
```

### 2.2 Variables

```
VarDecl = ( "let" | "const" ) Pattern [ ":" Type ] [ "=" Expr ] ;
Pattern = Ident | "_" | "{" PatField { "," PatField } [ "," ] "}" ;
PatField = Ident [ ":" Pattern ] ;                           (* struct destructuring *)
```

`const` is an immutable binding and requires an initialiser; `let` is mutable and, without an initialiser, requires a type annotation and definite assignment (checked). `var` does not exist. Module-level `let` is module-local state: it resets on hot reload (state that must survive lives in the host's state capability). Array destructuring `[a, b]` is reserved.

### 2.3 Functions

```
FnDecl     = [ "kernel" | "async" ] "function" Ident [ TypeParams ] "(" [ Params ] ")"
             [ ":" Type ] { Contract } Block ;                 (* "kernel", "async", Contract: reserved *)
Params     = Param { "," Param } [ "," ] ;
Param      = [ "inout" ] Ident [ ":" Type ] [ "=" Expr ] ;      (* "inout" reserved; type required in declarations *)
TypeParams = "<" TypeParam { "," TypeParam } [ "," ] ">" ;
TypeParam  = Ident [ "extends" Type ] ;
Contract   = ( "requires" | "ensures" ) Expr ;                  (* reserved *)
```

* Exported function signatures need explicit parameter and return types (local inference stops at module boundaries); a missing return type on a non-exported function is inferred from the body (the `void` default when no `return` carries a value).
* Default parameter values are evaluated at each call that omits the argument and must not reference other parameters.
* No rest parameters, no overloads, no `arguments`, no `this` rebinding.
* Closures: arrow functions capture variables by reference to a shared cell when the variable is mutated after capture and by value otherwise (compiler decision, not observable).

```
ArrowFn    = ( Ident | "(" [ Params ] ")" [ ":" Type ] ) "=>" ( Expr | Block ) ;
```

### 2.4 Structs, classes, interfaces

```
RecordDecl = ( "struct" | "class" ) Ident [ TypeParams ] [ "implements" TypeList ] "{" { Member } "}" ;
Member     = { DocComment } { Attribute } ( FieldDecl | MethodDecl | CtorDecl ) ;
FieldDecl  = { "private" | "readonly" | "static" } Ident ":" Type [ "=" Expr ] ( ";" | "," ) ;   (* the last terminator may be omitted *)
MethodDecl = { "private" | "static" | "mutating" } Ident [ TypeParams ] "(" [ Params ] ")"
             [ ":" Type ] Block ;
CtorDecl   = "constructor" "(" [ Params ] ")" Block ;            (* classes only *)

InterfaceDecl = "interface" Ident [ TypeParams ] [ "extends" TypeList ] "{" { MethodSig } "}" ;
MethodSig     = Ident [ TypeParams ] "(" [ Params ] ")" [ ":" Type ] ";" ;
TypeList      = Type { "," Type } ;
```

* **`struct`**: a *value type*. Assignment and argument passing have value semantics (copy-on-write under the hood); a struct has no identity and no `null` unless the type says `| null`. Created with an object literal whose type comes from context (`const p: Point = { x: 1.0, y: 2.0 }`) or by a function. Methods that assign to `this` fields must be declared `mutating`.
* **`class`**: a *reference type* (reference counted, identity, `===` compares identity). Created with `new Counter(0)`; fields are initialised by field initialisers and the constructor; all fields must be definitely assigned. No inheritance (`extends` on a class is `OXS-E0900`; use interfaces and composition).
* **Interfaces are structural for satisfaction, nominal for everything else:** a struct or class satisfies an interface if it has the required methods with matching signatures (no `implements` needed; `implements` is documentation that the checker verifies). In the MVP checker, interfaces are usable as **generic bounds** (`<T extends Shape>`, monomorphised); using an interface as a value type (dynamic dispatch) is parsed but gated until a later slice (the AST needs nothing new).
* No getters/setters, no parameter properties, no `abstract`, no decorators (`@name` is the attribute syntax of section 4).

### 2.5 Enums and unions

```
EnumDecl  = "enum" Ident [ ":" Type ] "{" EnumVar { "," EnumVar } [ "," ] "}" ;
EnumVar   = Ident [ "=" Expr ] ;                                   (* constant integer expression *)
TypeAlias = "type" Ident [ TypeParams ] "=" Type ";" ;
```

* **`enum`** is the C-like integer enumeration (default underlying `i32`, `enum Color: u8 { ... }`); members are `Color.Red`. It is not a sum type.
* **Sum types are TypeScript's discriminated unions**, spelled exactly as in TypeScript:
  ```ts
  type Shape =
    | { kind: "circle"; r: f64 }
    | { kind: "rect"; w: f64; h: f64 };
  ```
  Rules: every member is an object type (or a named struct type); all members share one **discriminant** field whose type is a distinct *literal type* (string, integer or bool literal) per member; the compiler lowers the union to a tagged heap object (see `abi.md`). Members get compiler names (`Shape#circle`) and can also be written as named aliases (`type Circle = { kind: "circle"; r: f64 }; type Shape = Circle | Rect;`). Narrowing: `switch (s.kind)`, `if (s.kind === "circle")`.
  **The discriminant is not stored data.** `kind` is derived from the union tag (the variant index in the ABI, `abi.md` 2.3): a union object holds the tag and the *other* fields only. Reading `s.kind` yields the literal of the variant the tag selects (as a `string`/integer/`bool` value; a table lookup or comparison chain generated by the compiler), `s.kind` is read-only, and an object literal `{ kind: "circle", r: 1 }` selects the variant (it sets the tag, stores `r`). Consequently `kind` never appears in a struct layout and a union member without other fields costs only the tag.
* `T | null` is the only other union form in the MVP (one absence value). `string | number` style unions are not supported (`OXS-E0100` suggests a discriminated union).
* **`Result<T, E>`** is not special syntax; it is the standard-library union
  ```ts
  type Result<T, E> = { kind: "ok"; value: T } | { kind: "err"; error: E };
  function Ok<T, E>(value: T): Result<T, E> { return { kind: "ok", value }; }
  function Err<T, E>(error: E): Result<T, E> { return { kind: "err", error }; }
  ```
  with one language hook: the postfix `?` (section 3.3).

### 2.6 Laws

```
LawDecl  = "law" Ident [ TypeParams ] "(" [ Params ] ")" [ "when" Expr ] LawBlock ;
LawBlock = "{" { Stmt } Expr "}" ;                                 (* the tail expression has no ";" *)
```

A law is a declaration of a property the program must satisfy; the body's tail expression is a `bool`. `when` restricts the inputs (a precondition: counterexamples must satisfy it). Laws are checked at three levels (tested in dev builds, solver-proven under the `safety` profile, hand/AI proofs only when the solver fails) described in `docs/oxi-ui.md` decision 11; the *syntax* is the same at every level. Law bodies may call any pure function; they may not use host capabilities. `law` is a contextual keyword: it starts an item only when followed by `Ident` and `(` or `<`.
```ts
law sorted_stays_sorted(xs: i32[]) {
  isSorted(sort(xs))
}
law add_commutes(a: i32, b: i32) when a > -1000 && b > -1000 { add(a, b) === add(b, a) }
```
The block is the only place a block ends in an expression; everywhere else a final expression without `;` is `OXS-E0004`.

### 2.7 Rust libraries (linked crates): a deliberate trade-off

Status: rule of the language from 2026-11-25 (owner decision); resolution, bindings and tooling are built in slice 8.N.

A program may use a Rust library through `import { Regex } from "rust:regex";`. The crate, compiled for `wasm32-unknown-unknown`, and a binding generated by `oxi-bindgen` are **statically linked into the program's own Wasm module**: one module, one linear memory, one allocator (Rust's global allocator is `oxi_alloc`), and callbacks from Rust into OxiScript closures are ordinary calls. This is the main way to get a working library before it has been ported to OxiScript; ports replace linked crates over time.

What a linked crate keeps and what it gives up. Using one is always the programmer's informed choice, never silent:

| Property | OxiScript code | Linked Rust crate |
|---|---|---|
| Host safety (sandbox, fuel, capabilities only) | yes | **yes**: the crate is inside the same sandbox, metered by the same fuel, and reaches the host only through capabilities |
| Memory safety *of the program itself* | yes (checked types, RC, verifier) | **only as good as the crate**: Rust can be written with `unsafe`, and a bug there can corrupt this program's heap (never the host) |
| Effects, laws, placement report, bug shapes (8.T, 8.S4) | yes | **no**: these analyses run on OxiScript code only; the crate's effects are what its binding declares |
| Determinism (canonical NaN, fixed iteration order) | yes (`ir.md` section 6) | **not guaranteed**: NaN bits inside the crate are not canonicalised; the toolchain fixes hash seeds, everything else is the crate's behaviour |
| Size | small | adds the crate's code (often tens to hundreds of KB): a consequence of the choice, not an error |
| Failure | trap with a reason | a Rust panic is `abort` on wasm32 = a trap of the instance |
| Borrows across callbacks | n/a | enforced by the Rust compiler in the binding: borrowing program data needs `&Oxi<'a>`, calling back into OxiScript needs `&mut Oxi<'a>`, so no Rust borrow survives a callback |

**The toolchain tells the user, every build.** Preparing a program that links crates prints a notice listing each crate (and its dependencies) with version, whether it contains `unsafe` code, and which rows of the table above it drops; the placement report carries the same list. A profile may forbid linked crates or crates with `unsafe`.

**Getting a crate.** Only crates that build for `wasm32-unknown-unknown` can be linked (no files, threads, sockets or OS clocks; those are capabilities). The documentation shows how to build any such crate yourself, and a set of ready-made, prebuilt bindings for common libraries is shipped (needed in the browser and on devices, where no `rustc` is available).

**Native libraries outside the sandbox** (`native` in the manifest) exist only for what cannot run in Wasm at all (operating-system services); they are chosen by the host operator, never by a script.

C and C++ libraries follow the same route (compiled to Wasm and linked, C++ exceptions disabled).

---

## 3. Types, statements, expressions

### 3.1 Type syntax

```
Type        = FnType | UnionType ;
FnType      = "(" [ FnTypeParam { "," FnTypeParam } ] ")" "=>" Type ;
FnTypeParam = Ident ":" Type ;
UnionType   = [ "|" ] ArrayType { "|" ArrayType } ;
ArrayType   = PrimaryType { "[" "]" } ;
PrimaryType = TypePath [ "<" TypeList ">" ] | LiteralType | "null" | ObjectType | "(" Type ")" ;
TypePath    = Ident { "." Ident } ;
LiteralType = STRING | INT | "true" | "false" ;                    (* discriminants only *)
ObjectType  = "{" [ FieldSig { ( ";" | "," ) FieldSig } [ ";" | "," ] ] "}" ;
FieldSig    = [ "readonly" ] Ident ":" Type ;
```
`T[]` and `Array<T>` are the same type. Anonymous object types are only legal as union members and alias right-hand sides (each becomes a named struct); there are no structural object types elsewhere. Reserved: tuples `[A, B]`, fixed arrays `[T; N]`, `dyn Interface`.

### 3.2 Statements

```
Stmt      = { Attribute } StmtBody ;
StmtBody  = VarDecl ";" | ExprStmt | IfStmt | WhileStmt | DoWhileStmt | ForStmt | ForOfStmt
          | SwitchStmt | "return" [ Expr ] ";" | "break" [ Ident ] ";" | "continue" [ Ident ] ";"
          | Block | Ident ":" Stmt | Item | ";" ;
Block     = "{" { Stmt } "}" ;
ExprStmt  = Expr ";" ;                                             (* assignment, call, ++/--, new, ... *)
IfStmt    = "if" "(" Expr ")" Stmt [ "else" Stmt ] ;
WhileStmt = "while" "(" Expr ")" Stmt ;
DoWhileStmt = "do" Stmt "while" "(" Expr ")" ";" ;
ForStmt   = "for" "(" ( VarDecl | [ Expr ] ) ";" [ Expr ] ";" [ Expr ] ")" Stmt ;
ForOfStmt = "for" "(" ( "const" | "let" ) Pattern "of" Expr ")" Stmt ;
SwitchStmt = "switch" "(" Expr ")" "{" { CaseClause } "}" ;
CaseClause = CaseLabel { CaseLabel } { Stmt } ;
CaseLabel  = "case" Expr { "," Expr } ":" | "default" ":" ;
```
* `ForOfStmt` iterates arrays and `range(a, b)`; the loop variable is a fresh binding per iteration. A C-style `for (let i = ...; ...; ...)` follows TypeScript/JavaScript (`CreatePerIterationEnvironment`): every iteration has its own binding of each `let` declared in the initialiser; the binding of the next iteration starts from the value of the previous one **before** the update expression runs, so a closure created in iteration *n* sees the value that iteration *n* left behind (`for (let i = 0; i < 3; i++) fs.push(() => i)` yields `0, 1, 2`), and a closure that assigns the variable changes only its own iteration's binding (and what the next iteration starts from, if it runs before the next iteration begins). `for (const k in obj)` does not exist.
* An expression statement must have an effect: a bare `a + b;` is a warning (`OXS-W0001` family); `i++` and `--i` are valid only as expression statements and `for` updates (they yield no value).
* **No fall-through.** Control never runs from one case clause into the next; a clause ends at the next `case`/`default`/`}`. `break` exits the switch early and is never required. Several labels stack on one clause (`case "a": case "b": ...` or `case "a", "b":`), which replaces TypeScript's empty-case fall-through idiom. A `default` may share a clause with other labels.
* **Termination analysis:** an exhaustive `switch` in which every clause ends in `return`/`break`-free terminating control flow (all clauses return, trap or `continue`) terminates the enclosing function body, so `area` below needs no final `return`; a function whose return type is not `void` must return on every path (`OXS-E0100`).
* **Exhaustiveness:** a `switch` over an `enum`, a discriminated union (`switch (s.kind)`), a `bool` or a `T | null` must cover every possibility or have `default`; otherwise `OXS-E0101` lists the missing cases. Cases narrow the scrutinee's owner (`s.r` is available under `case "circle":`).
* Attributes before a statement attach to it (`@expect(simd) for (...) ...`). Reserved: `parallel for`, `arena { }`, `trusted { }`.

### 3.3 Expressions

```
Expr       = Assign ;
Assign     = Cond [ AssignOp Assign ] ;                            (* target must be a place *)
AssignOp   = "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "+%=" | "-%=" | "*%=" | "+|=" | "-|=" | "*|="
           | "&=" | "|=" | "^=" | "<<=" | ">>=" ;
Cond       = Coalesce [ "?" Expr ":" Cond ] ;                      (* "?" preceded by whitespace, 1.5 *)
Coalesce   = Or { "??" Or } ;                                      (* mixing with || or && needs parentheses *)
Or         = And { "||" And } ;
And        = BitOr { "&&" BitOr } ;
BitOr      = BitXor { "|" BitXor } ;
BitXor     = BitAnd { "^" BitAnd } ;
BitAnd     = Equality { "&" Equality } ;
Equality   = Relational { ( "===" | "!==" ) Relational } ;
Relational = Shift { ( ( "<" | "<=" | ">" | ">=" ) Shift | "as" CastType ) } ;   (* "as" sits at this level, as in TS *)
CastType   = ArrayType ;                                            (* a non-union type; a union needs parentheses: x as (A | B) *)
Shift      = Additive { ( "<<" | ">>" ) Additive } ;
Additive   = Multiplicative { ( "+" | "-" | "+%" | "-%" | "+|" | "-|" ) Multiplicative } ;
Multiplicative = Unary { ( "*" | "/" | "%" | "*%" | "*|" ) Unary } ;
Unary      = ( "-" | "!" | "~" | "++" | "--" ) Unary | Postfix ;
Postfix    = Primary { "." Ident | "?." Ident | "[" Expr "]" | "?." "[" Expr "]"
                     | [ "<" TypeList ">" ] "(" [ Args ] ")" | "?." "(" [ Args ] ")"
                     | "?" (* try *) | "++" | "--" } ;
Args       = Expr { "," Expr } [ "," ] ;
Primary    = Literal | Ident | "this" | "(" Expr ")" | ArrayLit | ObjectLit | TEMPLATE
           | ArrowFn | "new" Type "(" [ Args ] ")" | SwitchExpr ;
ArrayLit   = "[" [ Expr { "," Expr } [ "," ] ] "]" ;
ObjectLit  = "{" [ ObjField { "," ObjField } [ "," ] ] "}" ;
ObjField   = Ident [ ":" Expr ] ;                                  (* shorthand { x } *)
SwitchExpr = "switch" "(" Expr ")" "{" Arm { Arm } "}" ;
Arm        = ( "case" Expr { "," Expr } | "default" ) ":" Expr [ "," ] ;   (* arms use ":" never an arrow; the last "," is optional *)
```
**Precedence and associativity are exactly TypeScript's** (including `as`, which sits at the relational level, so `a * b as T` is `(a * b) as T`). Loosest to tightest: assignment (right), `?:` (right), `??`, `||`, `&&`, `|`, `^`, `&`, `=== !==`, `< <= > >= as`, `<< >>`, `+ - +% -% +| -|`, `* / % *% *|`, prefix `- ! ~ ++ --`, postfix. All binary levels are left-associative; relational operators are non-associative.

**Must-parenthesise rules (syntax errors, never silently reinterpreted).** Where TypeScript/JavaScript parses an expression legally but it is a well-known trap, OxiScript keeps TS's parse tree for the parenthesised form and **rejects the unparenthesised mix**, so a TS developer never gets a different meaning, only an error with a fix-it that inserts the parentheses:
| Mix (direct operands, no parentheses) | Example | Code |
|---|---|---|
| bitwise `&` `\|` `^` with a comparison or equality operator | `a & b === c`, `x \| y < z` | `OXS-E0007` |
| shift `<<` `>>` with `+ - +% -% +\| -\|` | `a << b + c`, `x + y >> 1` | `OXS-E0008` |
| `??` with `\|\|` or `&&` | `a ?? b \|\| c` | `OXS-E0009` |
| chained relational / equality | `a < b < c`, `a === b === c` | `OXS-E0001` |
| `as` followed by `\|` | `x as u8 \| 1` | `OXS-E0010` |
| `&` `\|` `^` mixed with each other | `a & b \| c` | `OXS-E0007` |
Rationale: the first two are the classic JS precedence surprises (`x & 1 === 0` means `x & (1 === 0)`), and the third is already required by TS. An error keeps the TS reading available (add the parentheses you meant) while making a wrong guess impossible. Parenthesised operands are exempt, as is the same operator repeated (`a & b & c`, `a + b + c`). The formatter never removes parentheses required by these rules.

* **Casts:** `x as T` converts numerics losslessly or traps if the value is out of range (float to int also traps on NaN). There is no assertion-style cast: `x as T` never lies about a type. Spellings that glue a sign onto the keyword (the wrapping, saturating and downcast variants) would differ from `as` followed by an operator only by whitespace, so those conversions are **ordinary generic method calls**, not syntax: `x.wrapping<u8>()` (two's complement truncation), `x.saturating<u8>()` (clamp; NaN gives 0) and `x.downcast<Circle>()` (checked reference downcast, yields `Circle | null`). They are lang-item methods of the standard library, readable by any TS developer, parse with the generic-call rule of 3.4, and give the checker/IR a name to hang the `conv.wrap`/`conv.sat` semantics on. `as` takes a non-union type; see 3.4 rule 8 for `x as u8 | 1`.
* **Equality `===`/`!==`** is defined for numbers (IEEE for floats: `NaN !== NaN`), `bool`, `string` (by value), enums, `null` comparisons, and **classes by identity**. On a `struct`, a union, an array or a closure it is a compile error (`OXS-E0100`): structs have no identity under copy-on-write, so write the field comparison (or a function) instead. (A later edition may derive structural equality.)
* **Try operator `e?`:** `e` must have type `Result<T, E>` and the enclosing function must return `Result<_, E>` (the same `E` in the MVP): `e?` evaluates to the `value` if `ok`, otherwise returns the `err` from the function. Otherwise `OXS-E0104`.
* **Optional chaining and null:** `a?.b` is `null` when `a` is `null`, else `a.b`; `a ?? b` supplies a default. There is no `undefined`, no postfix `!` assertion and no `?.` on non-nullable types (`OXS-E0105`).
* **Object literals** `{ ... }` get their struct type from context (declared type, parameter type, return type, union expectation); with no context it is `OXS-E0103`. A `{` at the start of a statement is always a block; to start an expression statement with an object literal, parenthesise it. After `=>` a `{` is a block body; return an object literal as `=> ({ x: 1 })`.
* **`SwitchExpr` versus `SwitchStmt`:** position decides. A `switch` at statement start is the statement form; a `switch` anywhere an expression is expected (right of `=`, after `return`, as an argument, as an arrow body, ...) is an expression and every arm is `case a, b: expr,` or `default: expr,` with exactly one expression (to use a `switch` expression as an expression statement, parenthesise it). Arms deliberately use `:` and never `=>`: a label such as `case x => v` can therefore never be mistaken for an arrow function, and `case x => v` is **not an arrow function** here, it is a syntax error (`OXS-E0001`, fix: use `:`). A label that itself contains a top-level `?:` must be parenthesised (`case (a ? b : c):`). A `switch` expression must be exhaustive.
```ts
const name = switch (c) { case Color.Red: "red", case Color.Green, Color.Blue: "cool", };
```
* **`this`** is only valid in methods and constructors; it never rebinds.

### 3.4 Disambiguation rules for the parser (8.P)

1. **Generic call versus comparison (TypeScript's rule, restricted to calls):** after an expression, `<` begins type arguments iff a speculative parse of `"<" TypeList ">"` succeeds **and** the token after the closing `>` is `(`; otherwise `<` is the relational operator and the parser rewinds. This is what TypeScript does for calls (TS additionally allows tagged templates and instantiation expressions, which OxiScript lacks). Consequences, identical to TypeScript: `f<i32>(x)` is a generic call; `a < b > (c)` **is** the generic call `a<b>(c)`; `f(a < b, c > (d))` is a call with **one** argument, the generic call `a<b, c>(d)`. To compare, parenthesise: `(a < b) > c`, `f((a < b), (c > d))`. The formatter adds such parentheses when it would otherwise change the reading.
2. **Arrow function versus parenthesised expression:** at `(`, scan to the matching `)`; it is an arrow function iff the next token is `=>`, or `:` followed by a type and then `=>`. `Ident =>` is an arrow function with one parameter. **Inside the true branch of a ternary** (`c ? ( ... ) : T => body ...`) the second form is ambiguous with the ternary's own `:`; the rule is TypeScript's: `(...) : T => body` is an arrow function with a return type only if, after its body, the next token is `:` (the else separator); otherwise the `(...)` is a parenthesised expression, the `:` belongs to the ternary and `T => body` is the else branch. So `x ? (y) : z => w` is a ternary whose else branch is the arrow function `z => w`, and `x ? (y): T => y : z` is a ternary whose true branch is an arrow function.
3. **Block versus object literal:** `{` at the start of a statement is a block; `{` where an expression is expected is an object literal; `=> {` is a block body.
4. **`?`:** lexical rule of section 1.5; `?.` followed by an identifier, `[` or `(` is always optional chaining (also after whitespace or a line break).
5. **Switch:** position decides (3.3): at statement start the statement form (`case a: stmts`), elsewhere the expression form (`case a: expr,`). Statement-form clauses may stack labels (`case a: case b: stmts`) and contain statements; expression-form arms contain exactly one expression and end in `,`. Neither form uses `=>`.
6. **`type` and `law`** start an item only when followed by an identifier and `=`/`<` (`type`) or `(`/`<` (`law`); otherwise they are identifiers.
7. **`>>`** (and likewise `>=` and `>>=`) are split while parsing type arguments: after a type argument list the leading `>` is consumed and the rest becomes the next token (`Array<Array<i32>>` splits `>>`; `let a: Array<i32>= x` splits `>=` into `>` and `=`; `let b: Array<Array<i32>>= x` splits `>>=`).
8. **`as`** takes a **non-union** type (`CastType = ArrayType`): after `x as u8`, a following `|` is not part of the type. TypeScript would read `x as u8 | 1` as a cast to the union `u8 | 1`, while C-family readers see `(x as u8) | 1`; since the two disagree, OxiScript rejects the unparenthesised form with `OXS-E0010` and offers both fixes: `(x as u8) | 1` and `x as (u8 | 1)`. Type arguments in `as` types close greedily (`a as Array<T> < b`).
9. **Must-parenthesise mixes** (section 3.3 table) are reported by the parser with `OXS-E0007`/`E0008`/`E0009` (chained comparisons `OXS-E0001`), each carrying a fix that inserts parentheses around the tighter-binding pair of TS's parse.

---

## 4. Attributes

```
Attribute = "@" Ident [ AttrArgs ] ;
AttrArgs  = "(" [ AttrArg { "," AttrArg } [ "," ] ] ")" ;
AttrArg   = [ Ident ":" ] ( STRING | INT | "true" | "false" | Ident { "." Ident } ) ;
```
Attributes attach to the following item, member or statement; `@@name(...)` at the top of a file attaches to the module. Unknown attribute names are errors (typos must not be silent). Registered in the MVP (semantics owned by the named slice):

| Attribute | Meaning |
|---|---|
| `@expect(simd)`, `@expect(no_alloc)`, ... | performance/safety assertion; a failed assertion is a compile error with the placement reason code (8.S4, 8.3) |
| `@profile(realtime)` / `@@profile(safety)` | restriction set checked by the compiler (kernel is reserved) |
| `@allow("OXS-P0102", "reason")` | suppress one report/warning; the reason string is mandatory (`OXS-W0002`) |
| `@inline`, `@noinline` | optimisation hints |
| `@export_name("init")` | name of the Wasm export (default: the function name) |

---

## 5. Reserved syntax for future features (frozen shapes)

Nothing below is accepted by the MVP parser (it reports `OXS-E0900`), but the AST already has the slot, so adding it never reshapes the tree and the keywords are already reserved.

| Future feature | Reserved syntax | AST slot |
|---|---|---|
| Kernel profile (data-parallel functions, GPU) | `kernel function f(...)`, `@profile(kernel)` | `FnModifiers::kernel` |
| Parallel loops | `parallel for (const i of range(0, n)) { }` | `For{,Of}::parallel` |
| Async / concurrency | `async function`, `await e`, `spawn`, `isolate` | `FnModifiers::is_async` (+ new `ExprKind`) |
| Mutable value parameters | `function f(inout p: Point)` | `ParamMode::Inout` |
| Contracts | `function f(x: i32): i32 requires x > 0 ensures ...` | `FnDecl::contracts` |
| Arenas, trusted blocks | `arena { ... }`, `trusted { ... }` | new `StmtKind` |
| SIMD types | `Simd<f32, 4>` is an ordinary generic type (no syntax) | none |
| Tuples, fixed arrays, array patterns | `[A, B]`, `[T; N]`, `const [a, b] = ...` | new `TypeExprKind` / `PatternKind` |
| Dynamic interface values | `dyn Shape` | new `TypeExprKind` |
| Unicode identifiers | next edition | lexer only |

---

## 6. Deviations from TypeScript (reading stays free, writing may need a fix)

Each row: what a TS developer would write, what OxiScript requires, and why.

| # | TypeScript | OxiScript | Reason |
|---|---|---|---|
| 1 | `let n: number = 1;` | `let n: i32 = 1;` (or inferred `i32`/`f64`) | No `number`: integers and floats are different types, overflow is defined and traps. |
| 2 | `a == b` | `a === b` (`==` is `OXS-E0005`) | One equality, no coercion. A TS developer already writes `===`. |
| 3 | `x as Circle` (assertion) | `x.downcast<Circle>()` (checked, `Circle \| null`); numeric `x as i64` checks the range, `x.wrapping<u8>()` / `x.saturating<u8>()` for the other modes | Soundness: no cast may lie about a type. |
| 4 | `let v: string \| undefined; v?.length` | `let v: string \| null = null; v?.length` | One absence value. |
| 5 | `a ?? b \|\| c` | needs parentheses | Same as TS (it also rejects the mix). |
| 6 | `try { f() } catch (e) {}` / `throw` | `const v = f()?;` with `Result<T, E>`; traps for bugs | Errors are values; no exceptions in the runtime. |
| 7 | `switch` with fall-through, `default` optional | no fall-through, exhaustive over unions/enums | Soundness and determinism; stacking labels replaces the empty-case idiom. |
| 8 | `type Shape = {kind:"circle";r:number} \| ...` | identical, with `f64` | Kept exactly: discriminated unions are what TS developers already write. |
| 9 | `enum Color { Red }` (number/string mix) | `enum Color: u8 { Red }` (C-like integers only) | Sum types are the unions above. |
| 10 | `interface Point { x: number }` as a data shape, structural everywhere | `struct Point { x: f64; y: f64; }` for data; `interface` for behaviour, structural satisfaction | Nominal data gives direct field offsets; structural behaviour keeps TS-style duck typing for functions. |
| 11 | `class B extends A` | not supported; use interfaces/composition | No virtual dispatch cost or fragile base classes in the MVP. |
| 12 | `for (const k in obj)`, `var`, `with`, `eval`, `any`, `undefined`, `typeof x`, `instanceof` | reserved/removed (`OXS-E0900` with a pointer to the replacement) | No dynamic features. |
| 13 | ASI (optional semicolons) | `;` mandatory | Unambiguous grammar (also friendlier to AI tooling). |
| 16 | `i++` anywhere | statement/for-update only | No evaluation-order puzzles (determinism). |
| 17 | `Math.floor(a / b)`, `a / b` on numbers | integer `/` truncates toward zero and traps on zero; float `/` is IEEE | Defined behaviour for every operation. |
| 18 | `{ x: 1, y: 2 }` has an inferred anonymous type | needs a contextual struct type | Anonymous property bags would need dynamic layout. |
| 19 | `arr[i]` out of range gives `undefined` | traps `bounds` | Defined behaviour; no `undefined`. |
| 20 | `str.length`, `str[i]` (UTF-16) | `str.byteLength`, iteration by code point; no O(1) code-point index | UTF-8 strings. |

Side-by-side sample:

```ts
// TypeScript
type Shape = { kind: "circle"; r: number } | { kind: "rect"; w: number; h: number };
function area(s: Shape): number {
  switch (s.kind) {
    case "circle": return Math.PI * s.r * s.r;
    case "rect":   return s.w * s.h;
  }
}
```
```ts
// OxiScript: the same text with f64 (and no fall-through, exhaustiveness checked)
type Shape = { kind: "circle"; r: f64 } | { kind: "rect"; w: f64; h: f64 };
function area(s: Shape): f64 {
  switch (s.kind) {
    case "circle": return PI * s.r * s.r;
    case "rect":   return s.w * s.h;
  }
}
```
```ts
// Errors as values (TypeScript would throw)
function parsePort(text: string): Result<u16, string> {
  const n = parseInt(text)?;                         // returns early on Err
  if (n < 1 || n > 65535) { return Err("port out of range"); }
  return Ok(n as u16);
}
```

---

## 7. Decisions for the owner to look at

1. **Whitespace-sensitive `?`** (section 1.5): the only whitespace-significant rule in an otherwise whitespace-free grammar. Alternatives: a different spelling for the try operator (`expr!`?, `try expr`), or dropping the ternary. Chosen because `f()?` (Rust) and `a ? b : c` (TS) both read naturally.
2. **`===` only** (`==` rejected with a fix).
3. **Mandatory semicolons.**
4. **`struct` is a copy-on-write heap value** in the MVP (no scalar replacement yet); this affects performance, not syntax.
5. **Precedence is TypeScript's, with must-parenthesise errors** for bitwise-vs-comparison, shift-vs-additive and `??`-vs-`||`/`&&` mixes (section 3.3). Changed from the first draft at the lead's review.
6. **ASCII-only identifiers** in the MVP.
7. **Braces optional** after `if`/`while`/`for` (TS parity; the formatter adds them). Requiring them would remove goto-fail bugs at the cost of `if (x) return;` habits.
