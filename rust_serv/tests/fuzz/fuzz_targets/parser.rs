#![no_main]
use libfuzzer_sys::fuzz_target;
use rust_serv::http::parser::{RequestParser,parse_borrowed};
fuzz_target!(|input:&[u8]| {
    if let Ok(Some((request,n)))=parse_borrowed(input) {
        assert!(n>0 && n<=input.len());
        for (name,_)in request.headers.iter(){assert!(!name.is_empty());}
        assert!(request.body.len()<=rust_serv::http::parser::MAX_BODY);
    }
    let step=input.first().map_or(1,|b|usize::from(*b)%32+1);
    let mut parser=RequestParser::default();
    let mut base=0;
    let mut end=0;
    while end<input.len() {
        end=(end+step).min(input.len());
        match parser.parse_indices(&input[base..end]) {
            Ok(Some(plan))=>{
                let consumed=plan.consumed();assert!(consumed>0 && consumed<=end-base);
                plan.request(&input[base..end]).expect("validated plan must yield a valid view");
                // A plan must reject a smaller buffer rather than panic.
                let _=plan.request(&[]);
                base+=consumed;
            },
            Err(_)=>break,
            Ok(None)=>{},
        }
    }
    // Inconsistent API callers are errors, not panics or unchecked slicing.
    let mut parser=RequestParser::default();
    let _=parser.parse_indices(input);
    let _=parser.parse_indices(&[]);
});
