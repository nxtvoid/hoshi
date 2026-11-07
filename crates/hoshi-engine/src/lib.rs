//! Runtime loop: takes commands, lets the core decide events, persists them
//! through the store, then runs the resulting effects (provider turns, git).
