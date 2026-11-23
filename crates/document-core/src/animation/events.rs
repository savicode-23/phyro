use super::{valid_name, Clip};

impl Clip {
    pub fn add_event(&mut self, tick: u32, name: &str) -> Result<(), String> {
        if tick >= self.frames {
            return Err("Choose a frame inside this clip.".into());
        }
        let name = name.trim();
        if !valid_name(name) {
            return Err("Use an event name between 1 and 80 characters.".into());
        }
        let list = self.events.entry(tick).or_default();
        if list.len() >= 64 {
            return Err("A frame can contain up to 64 events.".into());
        }
        list.push(name.into());
        Ok(())
    }

    pub fn rename_event(&mut self, tick: u32, index: usize, new_name: &str) -> Result<(), String> {
        let name = new_name.trim();
        if !valid_name(name) {
            return Err("Use an event name between 1 and 80 characters.".into());
        }
        let old = self
            .events
            .get_mut(&tick)
            .and_then(|list| list.get_mut(index))
            .ok_or("Unknown event.")?;
        *old = name.to_owned();
        Ok(())
    }

    pub fn remove_event(&mut self, tick: u32, index: usize) -> Result<(), String> {
        let list = self
            .events
            .get_mut(&tick)
            .filter(|v| index < v.len())
            .ok_or("Unknown event.")?;
        list.remove(index);
        if list.is_empty() {
            self.events.remove(&tick);
        }
        Ok(())
    }

    pub fn move_event(&mut self, tick: u32, index: usize, dest: u32) -> Result<(), String> {
        let name = self
            .events
            .get(&tick)
            .and_then(|v| v.get(index))
            .ok_or("Unknown event.")?
            .clone();
        if dest != tick {
            self.add_event(dest, &name)?;
            self.remove_event(tick, index)?;
        }
        Ok(())
    }

    pub(super) fn validate_events(&self) -> Result<(), String> {
        if self.events.iter().all(|(tick, names)| {
            *tick < self.frames
                && !names.is_empty()
                && names.len() <= 64
                && names.iter().all(|n| valid_name(n))
        }) {
            Ok(())
        } else {
            Err(format!(
                "Clip '{}' has an invalid event name, count or frame.",
                self.name
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::AnimationProject;
    #[test]
    fn events_follow_copied_removed_and_inserted_drawing_ranges() {
        let mut p = AnimationProject::default();
        let c = &mut p.clips[0];
        c.add_event(1, "step").unwrap();
        c.add_event(1, "dust").unwrap();
        c.add_event(4, "land").unwrap();
        c.duplicate_range(1, 2).unwrap();
        assert_eq!(c.events[&1], ["step", "dust"]);
        assert_eq!(c.events[&3], c.events[&1]);
        assert_eq!(c.events[&6], ["land"]);
        c.delete_range(1, 2).unwrap();
        c.blank_frame(0).unwrap();
        c.move_event(2, 1, 0).unwrap();
        assert_eq!(c.events[&0], ["dust"]);
        assert_eq!(c.events[&2], ["step"]);
        assert_eq!(c.events[&5], ["land"]);
        let snapshot = c.clone();
        assert!(c.set_length(5).is_err());
        assert!(c.move_event(2, 0, 6).is_err());
        assert_eq!(*c, snapshot);
        p.validate().unwrap();
    }
}
