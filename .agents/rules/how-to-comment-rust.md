---
trigger: manual
---

Skill: Rust Comment Refactoring & Technical Writing StandardYou are an expert Rust software architect and a seasoned technical writer. Your task is to refactor all comments in the provided Rust codebase to meet professional, production-grade, and idiomatic Rust standards. You must remove "agent noise" (over-commenting, redundant explanations) and replace it with highly valuable, precise, and idiomatic documentation.1. Core Philosophy: Explain "Why", Not "What"Rule: Delete all comments that explain what the code is doing if it is already obvious from the syntax.Action: If a comment can be inferred by reading a clean line of Rust code, remove it.Focus: Use internal comments (//) exclusively to explain why a design decision was made, why a workaround is necessary, or to explain complex domain-specific logic.Examples of what to DELETE:// Increment the counter (when followed by counter += 1;)// Check if the list is empty (when followed by if list.is_empty() {)// Create a new vector (when followed by let mut vec = Vec::new();)2. Syntactic Rules for Rust CommentsUse the correct comment types strictly according to their semantic meaning in Rust://! Module-level Comments (Outer): Use only at the very top of a file or module (mod.rs, lib.rs, main.rs) to explain the high-level architecture, purpose, and responsibility of the module./// Item-level Comments (Inner): Use for all public functions, structs, enums, fields, and traits. Even for complex private items, use /// instead of //.// Internal Implementation Comments: Use sparingly inside function bodies.// SAFETY: Unsafe Block Justifications: Mandatory directly before any unsafe {} block.3. Structural Standards for Item Documentation (///)Every documentation comment for a function or method must follow this exact layout:One-line Summary: The first line must be a single, concise sentence in the active present tense (e.g., "Calculates the dot product...", not "This function will calculate..."), ending with a period.Blank Line: Leave exactly one empty line.Detailed Explanation (Optional): Provide a paragraph explaining the algorithm, mathematical context (use LaTeX notation like $O(N)$ for complexity), or architectural assumptions.Standard Headers: Use these specific Markdown headers only when applicable:# Examples: Include a practical, compilable doctest showing how to use the item.# Errors: Mandatory if the function returns a Result. Explain what conditions cause it to return Err.# Panics: Mandatory if the function can panic (via unwrap(), expect(), array indexing, or division by zero). Explain the boundary conditions that trigger a panic.# Safety: Strictly mandatory if the function is declared as unsafe fn. Explain the safety contracts the caller must manually uphold to avoid Undefined Behavior (UB).4. The Unsafe Code RuleWhenever you encounter or write an unsafe block, you must document it with a // SAFETY: comment directly preceding the block. It must explain why this specific invocation is guaranteed to be sound under Rust's memory safety rules.// SAFETY: The pointer is guaranteed to be non-null and points to a valid
// initialized block of memory of size `len`.
unsafe {
    std::slice::from_raw_parts(ptr, len);
}
5. Before & After Transformation Example🛑 BEFORE (Low-quality, noisy agent comments)// Struct for keeping track of user points
pub struct UserTracker {
    // The map storing user id and their points
    pub scores: HashMap<String, u32>,
}

impl UserTracker {
    // This function adds score to the user
    pub fn add_score(&mut self, user: &str, points: u32) -> Result<(), TrackerError> {
        // If the username is empty, we return an error
        if user.is_empty() {
            return Err(TrackerError::EmptyUsername);
        }
        
        // Find the entry and modify it or insert a default
        let entry = self.scores.entry(user.to_string()).or_insert(0);
        // Add points to the entry
        *entry += points;
        
        // Return Ok status
        Ok(())
    }
}
❇️ AFTER (Idiomatic, clean, professional Rust documentation)/// Tracks and manages active user score states.
pub struct UserTracker {
    /// Mapping of unique usernames to their accumulated points.
    pub scores: HashMap<String, u32>,
}

impl UserTracker {
    /// Adds points to a specified user's score.
    ///
    /// If the user does not exist in the registry, they will be initialized
    /// with the provided points.
    ///
    /// # Errors
    ///
    /// Returns [`TrackerError::EmptyUsername`] if the `user` string is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut tracker = UserTracker { scores: HashMap::new() };
    /// tracker.add_score("alice", 10).unwrap();
    /// assert_eq!(tracker.scores.get("alice"), Some(&10));
    /// ```
    pub fn add_score(&mut self, user: &str, points: u32) -> Result<(), TrackerError> {
        if user.is_empty() {
            return Err(TrackerError::EmptyUsername);
        }
        
        self.scores
            .entry(user.to_string())
            .and_modify(|current| *current = current.saturating_add(points))
            .or_insert(points);
        
        Ok(())
    }
}
6. Execution Instructions for the AgentAnalyze the provided file.Do not change the logic, variable names, or behavior of the code, except for cleanups that simplify the code so that comments can be deleted entirely (e.g., changing manual checks to idiomatic combinators if it improves readability).Strip out all redundant, obvious, or conversational comments inside the functions.Add rich, accurate, standard-compliant rustdoc comments (/// and //!) for all modules, structs, enums, traits, and functions.Ensure any returning Result, panicking code path, or unsafe block has its corresponding # Errors, # Panics, or # Safety / // SAFETY: documentation.