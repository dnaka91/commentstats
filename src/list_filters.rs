use tokei::LanguageType;

pub fn run() {
    for filter in LanguageType::list() {
        println!("{:?}", filter);
    }
}
