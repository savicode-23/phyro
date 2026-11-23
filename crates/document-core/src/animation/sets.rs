use super::{valid_name, AnimationProject, Clip};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashSet;

/// The permanent home for clips in projects created before animation sets.
pub const DEFAULT_ANIMATION_SET_ID: u64 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationSet {
    pub id: u64,
    pub name: String,
}

pub(super) fn default_id() -> u64 {
    DEFAULT_ANIMATION_SET_ID
}
pub(super) fn is_default_id(id: &u64) -> bool {
    *id == DEFAULT_ANIMATION_SET_ID
}
pub(super) fn first_unused_id() -> u64 {
    DEFAULT_ANIMATION_SET_ID + 1
}
pub(super) fn is_first_unused_id(id: &u64) -> bool {
    *id == first_unused_id()
}
pub(super) fn default_sets() -> Vec<AnimationSet> {
    vec![AnimationSet {
        id: DEFAULT_ANIMATION_SET_ID,
        name: "Animations".into(),
    }]
}
pub(super) fn is_default_collection(sets: &[AnimationSet]) -> bool {
    matches!(sets, [set] if set.id == DEFAULT_ANIMATION_SET_ID && set.name == "Animations")
}

impl AnimationProject {
    pub fn animation_set(&self, id: u64) -> Option<&AnimationSet> {
        self.animation_sets.iter().find(|set| set.id == id)
    }

    /// Label for clip choosers that show animations from several sets together.
    pub fn qualified_clip_name<'a>(&self, clip: &'a Clip) -> Cow<'a, str> {
        if self.animation_sets.len() > 1 {
            if let Some(set) = self.animation_set(clip.set_id) {
                return Cow::Owned(format!("{} / {}", set.name, clip.name));
            }
        }
        Cow::Borrowed(&clip.name)
    }

    /// Default-only archives remain readable by older versions of the editor.
    pub fn uses_animation_sets(&self) -> bool {
        !is_default_collection(&self.animation_sets)
            || !is_first_unused_id(&self.next_set_id)
            || self.clips.iter().any(|clip| !is_default_id(&clip.set_id))
    }

    pub(super) fn validate_animation_sets(&self) -> Result<(), String> {
        if self.animation_sets.len() > 256 {
            return Err("A project can have up to 256 animation sets.".into());
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for set in &self.animation_sets {
            if set.id == 0 || set.id >= self.next_set_id || !ids.insert(set.id) {
                return Err("An animation set has an invalid or duplicated persistent ID.".into());
            }
            if !valid_name(&set.name) || !names.insert(set.name.to_lowercase()) {
                return Err("Animation set names must be unique, with 1–80 characters.".into());
            }
        }
        if !ids.contains(&DEFAULT_ANIMATION_SET_ID) {
            return Err("The project is missing its default animation set.".into());
        }
        if self.clips.iter().any(|clip| !ids.contains(&clip.set_id)) {
            return Err("A clip belongs to an unknown animation set.".into());
        }
        Ok(())
    }

    fn check_set_name(&self, name: &str, skip: Option<u64>) -> Result<(), String> {
        if !valid_name(name) {
            return Err(
                "Enter an animation set name of 1–80 characters, without surrounding spaces."
                    .into(),
            );
        }
        let normalized = name.to_lowercase();
        if self
            .animation_sets
            .iter()
            .any(|set| Some(set.id) != skip && set.name.to_lowercase() == normalized)
        {
            return Err(format!("An animation set named '{name}' already exists."));
        }
        Ok(())
    }

    pub fn add_animation_set(&mut self, name: &str) -> Result<u64, String> {
        self.check_set_name(name, None)?;
        if self.animation_sets.len() >= 256 {
            return Err("The project already has 256 animation sets.".into());
        }
        let id = self.next_set_id;
        self.next_set_id = id
            .checked_add(1)
            .ok_or("Animation set IDs have been exhausted.")?;
        self.animation_sets.push(AnimationSet {
            id,
            name: name.into(),
        });
        Ok(id)
    }

    pub fn rename_animation_set(&mut self, id: u64, name: &str) -> Result<(), String> {
        self.check_set_name(name, Some(id))?;
        self.animation_sets
            .iter_mut()
            .find(|set| set.id == id)
            .ok_or("Unknown animation set.")?
            .name = name.into();
        Ok(())
    }

    /// On a name conflict, keep both the clip and its original membership intact.
    pub fn move_clip_to_set(&mut self, index: usize, set_id: u64) -> Result<(), String> {
        let clip = self.clips.get(index).ok_or("Unknown clip.")?;
        self.check_name(set_id, &clip.name, Some(index))?;
        self.clips[index].set_id = set_id;
        Ok(())
    }

    /// Move all members to the default set. Only colliding names receive a suffix.
    pub fn remove_animation_set(&mut self, id: u64) -> Result<(), String> {
        if id == DEFAULT_ANIMATION_SET_ID {
            return Err("Keep the default animation set so clips always have a home.".into());
        }
        let index = self
            .animation_sets
            .iter()
            .position(|set| set.id == id)
            .ok_or("Unknown animation set.")?;
        let existing: HashSet<_> = self
            .clips
            .iter()
            .filter(|c| c.set_id == DEFAULT_ANIMATION_SET_ID)
            .map(|c| c.name.to_lowercase())
            .collect();
        let mut reserved = existing.clone();
        reserved.extend(
            self.clips
                .iter()
                .filter(|c| c.set_id == id)
                .map(|c| c.name.to_lowercase()),
        );
        for clip in self.clips.iter_mut().filter(|c| c.set_id == id) {
            if existing.contains(&clip.name.to_lowercase()) {
                for number in 2.. {
                    let suffix = format!(" ({number})");
                    let prefix: String = clip.name.chars().take(80 - suffix.len()).collect();
                    let candidate = format!("{}{suffix}", prefix.trim_end());
                    if reserved.insert(candidate.to_lowercase()) {
                        clip.name = candidate;
                        break;
                    }
                }
            }
            clip.set_id = DEFAULT_ANIMATION_SET_ID;
        }
        self.animation_sets.remove(index);
        Ok(())
    }
}
