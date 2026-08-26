use convert_case::Casing;
use syn::ext::IdentExt;
use syn::Ident;

pub fn convert_from_snake_case(name: &Ident) -> Ident {
    let name_str = name.unraw().to_string();
    if !name_str.is_case(convert_case::Case::Snake) {
        name.unraw()
    } else {
        Ident::new(&name_str.to_case(convert_case::Case::Pascal), name.span())
    }
}
