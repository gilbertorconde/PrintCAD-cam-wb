//! A setup's run as its program cuts it: each operation at each part, in
//! the order the program takes them, and the tool's way from one part to
//! the next.
//!
//! Each part is cut the same in its own coordinates, so a section is its
//! operation's moves moved as far as its part is from the first. Between two
//! sections at different parts the program sends the tool clear and selects
//! the next offset; it is taken here as going straight up as high as either
//! end, across, and down, at the machine's rapids: the machine's own way up
//! is higher, so it takes a little longer than this.

use cam_protocol::Section;
use camcraft_post::profile::Rapid;
use glam::DVec3;

use crate::Drawn;

/// The run in sections, and how the tool gets to each.
#[derive(Debug, Clone, Default)]
pub(crate) struct Cut {
    pub sections: Vec<Section>,
    /// How far each part is from the first, along the setup's axes.
    pub parts: Vec<DVec3>,
    /// For each section, the tool's way to it from the last one cut at
    /// another part, in the setup's frame: each point and when the tool is
    /// there. Empty where it does not change parts.
    pub links: Vec<Vec<(DVec3, f64)>>,
}

impl Cut {
    /// `order`'s sections ([`camcraft_session::cut_order`]), each operation
    /// as `draws` has it, at `parts` (each part's offset), the tool moving
    /// between parts at `rapid`.
    pub fn of(order: &[(usize, usize)], draws: &[Drawn], parts: &[DVec3], rapid: &Rapid) -> Self {
        let mut out = Self {
            parts: parts.to_vec(),
            ..Self::default()
        };
        let mut clock = 0.0;
        // Where the tool last stood, and at which part.
        let mut last: Option<(DVec3, usize)> = None;
        for &(op, part) in order {
            let Some(drawn) = draws.get(op) else {
                continue;
            };
            let at = parts.get(part).copied().unwrap_or(DVec3::ZERO);
            let mut link = Vec::new();
            if let (Some((from, was)), Some(to)) = (last, drawn.ends.first())
                && was != part
            {
                let to = *to + at;
                let top = from.z.max(to.z);
                let way = [from, from.with_z(top), DVec3::new(to.x, to.y, top), to];
                link.push((way[0], clock));
                for leg in way.windows(2) {
                    clock += rapid.seconds(leg[1] - leg[0]);
                    link.push((leg[1], clock));
                }
            }
            let seconds = drawn.times.last().copied().unwrap_or(0.0);
            out.sections.push(Section {
                op,
                part,
                start: clock,
                seconds,
            });
            out.links.push(link);
            clock += seconds;
            if let Some(end) = drawn.ends.last() {
                last = Some((*end + at, part));
            }
        }
        out
    }

    /// When the whole run is done, in machining seconds.
    pub fn total(&self) -> f64 {
        self.sections.last().map_or(0.0, |s| s.start + s.seconds)
    }

    /// How far section `s`'s part is from the first's.
    pub fn offset(&self, s: usize) -> DVec3 {
        self.sections
            .get(s)
            .and_then(|s| self.parts.get(s.part))
            .copied()
            .unwrap_or(DVec3::ZERO)
    }

    /// The section under way at `seconds`: the last started at or before
    /// it, else the first.
    pub fn section_at(&self, seconds: f64) -> usize {
        self.sections
            .iter()
            .rposition(|s| s.start <= seconds)
            .unwrap_or(0)
    }

    /// Where the tool is at `seconds`, in the setup's frame, while it goes
    /// to section `s` from another part; `None` when it is not on the way
    /// there.
    pub fn between(&self, s: usize, seconds: f64) -> Option<DVec3> {
        let link = self.links.get(s)?;
        let (first, last) = (link.first()?, link.last()?);
        if seconds < first.1 || seconds >= last.1 {
            return None;
        }
        let i = link.iter().rposition(|(_, t)| *t <= seconds)?;
        let ((a, ta), (b, tb)) = (link[i], *link.get(i + 1)?);
        let f = if tb > ta {
            (seconds - ta) / (tb - ta)
        } else {
            1.0
        };
        Some(a.lerp(b, f))
    }

    /// For each part, the last section cut at it by section `s` (`s`
    /// itself for its own part): `None` for a part not started.
    pub fn standing(&self, s: usize) -> Vec<Option<usize>> {
        (0..self.parts.len().max(1))
            .map(|p| self.sections.iter().take(s + 1).rposition(|x| x.part == p))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(ends: Vec<DVec3>, times: Vec<f64>) -> Drawn {
        Drawn {
            strokes: Vec::new(),
            points: ends.len(),
            cut: 0.0,
            rapid: 0.0,
            after_move: (1..=ends.len()).collect(),
            ends,
            times,
            kept: None,
            z: None,
            fastest: 0.0,
        }
    }

    /// Two operations at two parts 100 mm apart, by tool: each operation at
    /// both before the next; the tool goes up, across and down between
    /// parts, at the rapids, and not between operations at one part.
    #[test]
    fn the_tool_goes_from_part_to_part_between_sections() {
        let draws = [
            drawn(
                vec![DVec3::new(0.0, 0.0, 5.0), DVec3::new(10.0, 0.0, 1.0)],
                vec![0.0, 10.0],
            ),
            drawn(
                vec![DVec3::new(0.0, 0.0, 5.0), DVec3::new(0.0, 10.0, 5.0)],
                vec![0.0, 20.0],
            ),
        ];
        let parts = [DVec3::ZERO, DVec3::new(100.0, 0.0, 0.0)];
        let rapid = Rapid {
            x: 6000.0,
            y: 6000.0,
            z: 600.0,
        };
        let cut = Cut::of(&[(0, 0), (0, 1), (1, 0), (1, 1)], &draws, &parts, &rapid);
        let starts: Vec<(usize, usize)> = cut.sections.iter().map(|s| (s.op, s.part)).collect();
        assert_eq!(starts, [(0, 0), (0, 1), (1, 0), (1, 1)]);
        // From (10, 0, 1) up 4 mm, across 90 mm and down none.
        let link = 4.0 / 600.0 * 60.0 + 90.0 / 6000.0 * 60.0;
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(cut.sections[1].start, 10.0 + link));
        assert!(cut.links[1].len() == 4 && cut.links[0].is_empty());
        // Back to the first part, from (110, 0, 1) to (0, 0, 5).
        let back = 4.0 / 600.0 * 60.0 + 110.0 / 6000.0 * 60.0;
        assert!(near(cut.sections[2].start, 20.0 + link + back));
        // And to the second again, from (0, 10, 5) to (100, 0, 5): across.
        assert!(near(
            cut.total(),
            60.0 + link + back + 100.0 / 6000.0 * 60.0
        ));
        // Half way up, at the second part's way there.
        let up = cut.between(1, 10.0 + 0.2).unwrap();
        assert!(near(up.x, 10.0) && near(up.z, 3.0), "{up}");
        assert_eq!(cut.between(1, 5.0), None);
        assert_eq!(cut.section_at(10.0 + 0.2), 0, "on the way, not there");
        assert_eq!(cut.section_at(cut.sections[1].start), 1);
        // By the third section the second part has had the first operation.
        assert_eq!(cut.standing(2), [Some(2), Some(1)]);
        assert_eq!(cut.standing(0), [Some(0), None]);
    }
}
