//! Notes and their comment threads as scene entities (ADR 0005).
//!
//! An annotation is an entity with an `Anchor` in world millimetres, so it means the same point
//! in every view and for every viewer. Comments are separate entities that name their annotation
//! by `EntityId` and are never edited or removed on their own: posting appends.

use hecs::{Entity, World};
use uuid::Uuid;

/// Stable identity of an entity that can be referred to from outside the running app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId(pub Uuid);

/// A point in world millimetres (the image affine's world).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub world_mm: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Label(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct Text(pub String);

/// Who created something and when. The author is a display name; there are no accounts yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Provenance {
    pub author: String,
    /// Milliseconds since the Unix epoch.
    pub created_at_ms: u64,
}

impl Provenance {
    pub fn now(author: &str) -> Self {
        let created_at_ms = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        Self {
            author: author.to_string(),
            created_at_ms,
        }
    }
}

/// Marks a comment as belonging to the annotation with this id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thread(pub Uuid);

/// One comment of a thread, read out of the world for display.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentView {
    pub author: String,
    pub text: String,
    pub created_at_ms: u64,
}

/// Creates a note at `world_mm` and returns its id.
pub fn spawn_annotation(
    world: &mut World,
    world_mm: [f32; 3],
    label: String,
    author: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    world.spawn((
        EntityId(id),
        Anchor { world_mm },
        Label(label),
        Text(String::new()),
        Provenance::now(author),
    ));
    id
}

/// One annotation as listed in the panels.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationRow {
    pub id: Uuid,
    pub label: String,
    pub note: String,
    pub anchor_mm: [f32; 3],
}

/// All annotations, oldest first.
pub fn annotation_rows(world: &World) -> Vec<AnnotationRow> {
    let mut rows: Vec<(u64, AnnotationRow)> = world
        .query::<(&EntityId, &Anchor, &Label, &Text, &Provenance)>()
        .iter()
        .map(|(_, (id, anchor, label, text, provenance))| {
            (
                provenance.created_at_ms,
                AnnotationRow {
                    id: id.0,
                    label: label.0.clone(),
                    note: text.0.clone(),
                    anchor_mm: anchor.world_mm,
                },
            )
        })
        .collect();
    rows.sort_by_key(|(created_at_ms, _)| *created_at_ms);
    rows.into_iter().map(|(_, row)| row).collect()
}

pub fn find_annotation(world: &World, id: Uuid) -> Option<Entity> {
    world
        .query::<(&EntityId, &Anchor)>()
        .iter()
        .find_map(|(entity, (entity_id, _))| (entity_id.0 == id).then_some(entity))
}

/// Appends a comment to the annotation's thread; `None` when the annotation does not exist.
pub fn add_comment(
    world: &mut World,
    annotation: Uuid,
    text: String,
    author: &str,
) -> Option<Uuid> {
    find_annotation(world, annotation)?;
    let id = Uuid::new_v4();
    world.spawn((
        EntityId(id),
        Thread(annotation),
        Text(text),
        Provenance::now(author),
    ));
    Some(id)
}

/// The annotation's comments, oldest first.
pub fn comments_of(world: &World, annotation: Uuid) -> Vec<CommentView> {
    let mut comments: Vec<CommentView> = world
        .query::<(&Thread, &Text, &Provenance)>()
        .iter()
        .filter(|(_, (thread, _, _))| thread.0 == annotation)
        .map(|(_, (_, text, provenance))| CommentView {
            author: provenance.author.clone(),
            text: text.0.clone(),
            created_at_ms: provenance.created_at_ms,
        })
        .collect();
    comments.sort_by_key(|comment| comment.created_at_ms);
    comments
}

/// Removes an annotation together with its thread.
pub fn delete_annotation(world: &mut World, id: Uuid) {
    let mut doomed: Vec<Entity> = world
        .query::<&Thread>()
        .iter()
        .filter(|(_, thread)| thread.0 == id)
        .map(|(entity, _)| entity)
        .collect();
    doomed.extend(find_annotation(world, id));
    for entity in doomed {
        let _ = world.despawn(entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comments_append_to_their_own_thread_only() {
        let mut world = World::new();
        let first = spawn_annotation(&mut world, [1.0, 2.0, 3.0], "a".into(), "Ann");
        let second = spawn_annotation(&mut world, [4.0, 5.0, 6.0], "b".into(), "Ann");

        add_comment(&mut world, first, "one".into(), "Bo").unwrap();
        add_comment(&mut world, first, "two".into(), "Cy").unwrap();
        add_comment(&mut world, second, "other".into(), "Bo").unwrap();

        let texts: Vec<_> = comments_of(&world, first)
            .into_iter()
            .map(|comment| (comment.author, comment.text))
            .collect();
        assert_eq!(
            texts,
            vec![("Bo".into(), "one".into()), ("Cy".into(), "two".into())]
        );
        assert_eq!(comments_of(&world, second).len(), 1);
    }

    #[test]
    fn test_rows_list_annotations_with_their_anchor_and_text() {
        let mut world = World::new();
        let id = spawn_annotation(&mut world, [1.0, 2.0, 3.0], "Lesion".into(), "Ann");
        let entity = find_annotation(&world, id).unwrap();
        world.get::<&mut Text>(entity).unwrap().0 = "check margin".into();

        let rows = annotation_rows(&world);

        assert_eq!(
            rows,
            vec![AnnotationRow {
                id,
                label: "Lesion".into(),
                note: "check margin".into(),
                anchor_mm: [1.0, 2.0, 3.0],
            }]
        );
    }

    #[test]
    fn test_a_comment_needs_an_existing_annotation() {
        let mut world = World::new();
        assert!(add_comment(&mut world, Uuid::new_v4(), "x".into(), "Bo").is_none());
        assert_eq!(world.len(), 0);
    }

    #[test]
    fn test_deleting_an_annotation_removes_its_thread_and_nothing_else() {
        let mut world = World::new();
        let gone = spawn_annotation(&mut world, [0.0; 3], "gone".into(), "Ann");
        let kept = spawn_annotation(&mut world, [1.0; 3], "kept".into(), "Ann");
        add_comment(&mut world, gone, "c".into(), "Bo").unwrap();
        add_comment(&mut world, kept, "c".into(), "Bo").unwrap();

        delete_annotation(&mut world, gone);

        assert!(find_annotation(&world, gone).is_none());
        assert!(find_annotation(&world, kept).is_some());
        assert!(comments_of(&world, gone).is_empty());
        assert_eq!(comments_of(&world, kept).len(), 1);
        assert_eq!(world.len(), 2);
    }

    #[test]
    fn test_provenance_records_author_and_a_time() {
        let provenance = Provenance::now("Ann");
        assert_eq!(provenance.author, "Ann");
        assert!(provenance.created_at_ms > 0);
    }
}
