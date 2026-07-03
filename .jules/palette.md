## 2024-07-03 — Ensure clear keyboard focus on interactive custom buttons
Learning: The sidebar has many custom buttons without explicitly clear focus indicators for keyboard users. Adding focus-visible rings is necessary for a11y.
Action: Add focus-visible utilities to custom unstyled buttons to match UI standards. Adding aria-current="page" to active items is also a good a11y practice.
