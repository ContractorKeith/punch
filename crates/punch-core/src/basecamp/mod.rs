//! External Basecamp boundary; authentication belongs to the installed CLI.
use crate::model::BasecampLink;

pub trait Basecamp {
    fn create(&self, link: &BasecampLink, column: &str, title: &str) -> Result<String, String>;
    fn move_card(&self, link: &BasecampLink, id: &str, column: &str) -> Result<(), String>;
    fn update_title(&self, link: &BasecampLink, id: &str, title: &str) -> Result<(), String>;
}

mod real;
pub use real::RealBasecamp;
