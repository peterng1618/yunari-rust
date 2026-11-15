#![forbid(unsafe_code)]
//! The editor's design tokens: one owner for spacing, radius and type scale.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RadiusScale {
    pub sm: u16,
    pub md: u16,
    pub lg: u16,
    pub xl: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeStep {
    pub name: &'static str,
    pub size_px: u16,
    /// Line height × 10, so 12 means 1.2.
    pub line_height_tenths: u16,
}

pub const SPACING_STEPS_PX: [u16; 10] = [4, 6, 8, 10, 12, 14, 16, 20, 24, 32];

pub const RADIUS_PX: RadiusScale = RadiusScale {
    sm: 4,
    md: 6,
    lg: 8,
    xl: 10,
};

pub const TYPE_SCALE: [TypeStep; 6] = [
    TypeStep {
        name: "2xs",
        size_px: 10,
        line_height_tenths: 12,
    },
    TypeStep {
        name: "xs",
        size_px: 11,
        line_height_tenths: 13,
    },
    TypeStep {
        name: "sm",
        size_px: 12,
        line_height_tenths: 14,
    },
    TypeStep {
        name: "md",
        size_px: 13,
        line_height_tenths: 13,
    },
    TypeStep {
        name: "lg",
        size_px: 15,
        line_height_tenths: 13,
    },
    TypeStep {
        name: "xl",
        size_px: 19,
        line_height_tenths: 12,
    },
];

pub const FONT_UI: &str = "system-ui";
pub const FONT_MONO: &str = "ui-monospace";

pub fn spacing_is_allowed(value: u16) -> bool {
    SPACING_STEPS_PX.contains(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_scale_is_the_only_scale() {
        assert_eq!(SPACING_STEPS_PX, [4, 6, 8, 10, 12, 14, 16, 20, 24, 32]);
        assert!(spacing_is_allowed(16));
        assert!(!spacing_is_allowed(7));
    }

    #[test]
    fn radius_scale_matches_the_design_document() {
        assert_eq!(
            RADIUS_PX,
            RadiusScale {
                sm: 4,
                md: 6,
                lg: 8,
                xl: 10
            }
        );
    }

    #[test]
    fn type_scale_matches_the_design_document() {
        let actual: Vec<(&str, u16, u16)> = TYPE_SCALE
            .iter()
            .map(|step| (step.name, step.size_px, step.line_height_tenths))
            .collect();
        assert_eq!(
            actual,
            vec![
                ("2xs", 10, 12),
                ("xs", 11, 13),
                ("sm", 12, 14),
                ("md", 13, 13),
                ("lg", 15, 13),
                ("xl", 19, 12),
            ]
        );
    }
}
