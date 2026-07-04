## 2025-05-18 - Avoid repeated array calculations inside React mapping loops
**Learning:** In complex mapping loops within React, doing operations that calculate data across an entire array on every iteration (e.g., finding the `Math.max` for a progress bar ratio using `Math.max(...data.map(d => d.value))`) escalates the rendering complexity to O(N²), causing serious slowdowns when the array scales up in components like `AnalyticsView.tsx`.
**Action:** Use an Immediately Invoked Function Expression (IIFE) around the block, or pre-calculate variables utilizing `useMemo` before mapping. This allows computing single values once before entering the loop to ensure strict O(N) array mapping performance.

## 2025-05-18 - Avoid redundant string parsing inside Array iteration methods
**Learning:** In mapping and finding loops (like `Array.prototype.find()` or `Array.prototype.map()`), putting string parsing logic (like `.split(',')` or `.trim().toLowerCase()`) directly inside the iterator callback can cause the Javascript engine to repeatedly allocate strings and arrays O(N) times for each item in the collection, creating noticeable garbage collection pauses.
**Action:** Always hoist parsing logic to construct validation criteria outside of the loop (e.g. constructing a Javascript `Set` once), then execute strict O(1) validations like `Set.has(item)` inside the loop body.
