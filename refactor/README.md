# Refactor Plans

This folder contains detailed refactor plans for distinct architectural improvements.

| Document | Area | Effort |
|----------|------|--------|
| [01_cleanup.md](./01_cleanup.md) | Dead code, Clippy warnings | 30 min |
| [02_overlay_system.md](./02_overlay_system.md) | Overlay abstraction | 1-2 hrs |
| [03_systems_split.md](./03_systems_split.md) | Split systems.rs | 2-3 hrs |
| [04_ecs_singletons.md](./04_ecs_singletons.md) | Singleton pattern | 4+ hrs |
| [05_annotations.md](./05_annotations.md) | Fix annotation bugs | 2-3 hrs |

## Execution Order

1. **01_cleanup** - Low risk, immediate gains
2. **02_overlay_system** - New abstraction, enables future features
3. **03_systems_split** - Improves maintainability
4. **04_ecs_singletons** - Larger refactor, optional
