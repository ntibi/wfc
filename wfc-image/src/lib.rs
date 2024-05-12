pub use coord_2d::{Coord, Size};
use grid_2d::Grid;
use image::{DynamicImage, Rgba, RgbaImage};
use rand::{Rng, SeedableRng};
use std::num::NonZeroU32;
use wfc::orientation::OrientationTable;
pub use wfc::orientation::{self, Orientation};
use wfc::overlapping::{OverlappingPatterns, Pattern};
use wfc::retry as wfc_retry;
pub use wfc::wrap;
pub use wfc::ForbidNothing;
use wfc::*;
pub use wrap::WrapXY;

pub mod retry {
    #[cfg(feature = "parallel")]
    pub use super::wfc_retry::ParNumTimes;
    pub use super::wfc_retry::RetryOwn as Retry;
    pub use super::wfc_retry::{Forever, NumTimes};

    pub trait ImageRetry: Retry {
        type ImageReturn;
        #[doc(hidden)]
        fn image_return(
            r: Self::Return,
            image_patterns: &super::ImagePatterns,
        ) -> Self::ImageReturn;
    }
}

pub struct ImagePatterns {
    pub overlapping_patterns: OverlappingPatterns<Rgba<u8>>,
    empty_colour: Rgba<u8>,
}

impl ImagePatterns {
    pub fn new(
        image: &DynamicImage,
        pattern_size: NonZeroU32,
        orientations: &[Orientation],
    ) -> Self {
        let rgba_image = image.to_rgba8();
        let size = Size::new(rgba_image.width(), rgba_image.height());
        let grid = Grid::new_fn(size, |Coord { x, y }| {
            *rgba_image.get_pixel(x as u32, y as u32)
        });
        let overlapping_patterns =
            OverlappingPatterns::new(grid, pattern_size, orientations);
        Self {
            overlapping_patterns,
            empty_colour: Rgba([0, 0, 0, 0]),
        }
    }

    pub fn set_empty_colour(&mut self, empty_colour: Rgba<u8>) {
        self.empty_colour = empty_colour;
    }

    pub fn image_from_wave(&self, wave: &Wave) -> DynamicImage {
        let size = wave.grid().size();
        let mut rgba_image = RgbaImage::new(size.width(), size.height());
        wave.grid().enumerate().for_each(|(Coord { x, y }, cell)| {
            let colour = match cell.chosen_pattern_id() {
                Ok(pattern_id) => {
                    *self.overlapping_patterns.pattern_top_left_value(pattern_id)
                }
                Err(_) => self.empty_colour,
            };
            rgba_image.put_pixel(x as u32, y as u32, colour);
        });
        DynamicImage::ImageRgba8(rgba_image)
    }

    pub fn weighted_average_colour<'a>(&self, cell: &'a WaveCellRef<'a>) -> Rgba<u8> {
        use wfc::EnumerateCompatiblePatternWeights::*;
        match cell.enumerate_compatible_pattern_weights() {
            MultipleCompatiblePatternsWithoutWeights | NoCompatiblePattern => {
                self.empty_colour
            }
            SingleCompatiblePatternWithoutWeight(pattern_id) => {
                *self.overlapping_patterns.pattern_top_left_value(pattern_id)
            }
            CompatiblePatternsWithWeights(iter) => {
                let (r, g, b, a) = iter
                    .map(|(pattern_id, weight)| {
                        let &Rgba([r, g, b, a]) =
                            self.overlapping_patterns.pattern_top_left_value(pattern_id);
                        (
                            r as u32 * weight,
                            g as u32 * weight,
                            b as u32 * weight,
                            a as u32 * weight,
                        )
                    })
                    .fold(
                        (0, 0, 0, 0),
                        |(acc_r, acc_g, acc_b, acc_a), (r, g, b, a)| {
                            (acc_r + r, acc_g + g, acc_b + b, acc_a + a)
                        },
                    );
                let total_weight = cell.sum_compatible_pattern_weight();
                Rgba([
                    (r / total_weight) as u8,
                    (g / total_weight) as u8,
                    (b / total_weight) as u8,
                    (a / total_weight) as u8,
                ])
            }
        }
    }

    pub fn grid(&self) -> &Grid<Rgba<u8>> {
        self.overlapping_patterns.grid()
    }

    pub fn id_grid(&self) -> Grid<OrientationTable<PatternId>> {
        self.overlapping_patterns.id_grid()
    }

    pub fn id_grid_original_orientation(&self) -> Grid<PatternId> {
        self.overlapping_patterns.id_grid_original_orientation()
    }

    pub fn pattern(&self, pattern_id: PatternId) -> &Pattern {
        self.overlapping_patterns.pattern(pattern_id)
    }

    pub fn pattern_mut(&mut self, pattern_id: PatternId) -> &mut Pattern {
        self.overlapping_patterns.pattern_mut(pattern_id)
    }

    pub fn global_stats(&self) -> GlobalStats {
        self.overlapping_patterns.global_stats()
    }

    pub fn collapse_wave_retrying<W, F, RT, R>(
        &self,
        output_size: Size,
        wrap: W,
        forbid: F,
        retry: RT,
        rng: &mut R,
    ) -> RT::Return
    where
        W: Wrap,
        F: ForbidPattern + Send + Sync + Clone,
        RT: retry::Retry,
        R: Rng + Send + Sync + Clone,
    {
        let global_stats = self.global_stats();
        let run = RunOwn::new_wrap_forbid(output_size, &global_stats, wrap, forbid, rng);
        run.collapse_retrying(retry, rng)
    }
}

impl retry::ImageRetry for retry::Forever {
    type ImageReturn = DynamicImage;
    fn image_return(
        r: Self::Return,
        image_patterns: &ImagePatterns,
    ) -> Self::ImageReturn {
        image_patterns.image_from_wave(&r)
    }
}

impl retry::ImageRetry for retry::NumTimes {
    type ImageReturn = Result<DynamicImage, PropagateError>;
    fn image_return(
        r: Self::Return,
        image_patterns: &ImagePatterns,
    ) -> Self::ImageReturn {
        match r {
            Ok(r) => Ok(image_patterns.image_from_wave(&r)),
            Err(e) => Err(e),
        }
    }
}

#[cfg(feature = "parallel")]
impl retry::ImageRetry for retry::ParNumTimes {
    type ImageReturn = Result<DynamicImage, PropagateError>;
    fn image_return(
        r: Self::Return,
        image_patterns: &ImagePatterns,
    ) -> Self::ImageReturn {
        match r {
            Ok(r) => Ok(image_patterns.image_from_wave(&r)),
            Err(e) => Err(e),
        }
    }
}

pub fn generate_image_with_rng<W, F, IR, R>(
    image: &DynamicImage,
    pattern_size: NonZeroU32,
    output_size: Size,
    orientations: &[Orientation],
    wrap: W,
    forbid: F,
    retry: IR,
    rng: &mut R,
) -> IR::ImageReturn
where
    W: Wrap,
    F: ForbidPattern + Send + Sync + Clone,
    IR: retry::ImageRetry,
    R: Rng + Send + Sync + Clone,
{
    let image_patterns = ImagePatterns::new(image, pattern_size, orientations);
    IR::image_return(
        image_patterns.collapse_wave_retrying(output_size, wrap, forbid, retry, rng),
        &image_patterns,
    )
}

pub fn generate_image<W, F, IR>(
    image: &DynamicImage,
    pattern_size: NonZeroU32,
    output_size: Size,
    orientations: &[Orientation],
    wrap: W,
    forbid: F,
    retry: IR,
) -> IR::ImageReturn
where
    W: Wrap,
    F: ForbidPattern + Send + Sync + Clone,
    IR: retry::ImageRetry,
{
    generate_image_with_rng(
        image,
        pattern_size,
        output_size,
        orientations,
        wrap,
        forbid,
        retry,
        &mut rand::rngs::StdRng::from_entropy(),
    )
}
