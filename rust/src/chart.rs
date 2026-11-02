//! The outer shell of a chart file: header, checksum, tile directory, records.
//! The format is documented in docs/CHART_FILES.md sections 1 to 4.

use anyhow::{bail, ensure, Result};
use md5::{Digest, Md5};

use crate::lzma;
use crate::pc1;

pub const MAGIC: u32 = 0x1B62;
pub const SECRET: &[u8] = b"d8ethebrestezexaqathaTrepedEkubafr5vuhaprupe3ucUphedeyuhaGespenU";
pub const HEAD: usize = 0x159C; // tile head size; records start here
pub const BLOB: usize = 0x157C; // the tile's 32-byte record key, PC1 encrypted

/// bikenav.exe FUN_002050a0: device IDs with one of these prefixes use the
/// built-in key, every other one a key derived from the ID itself.
pub const STATIC_KEY: &[u8] = b"E89ACE5CE51E0669B4BA068CE8F63990";
const KNOWN_PREFIXES: [&[u8; 8]; 19] = [
    b"20130125", b"20130212", b"20130213", b"20130807", b"20131010", b"20131020", b"20131026",
    b"20150215", b"20160505", b"20160509", b"20161014", b"20161028", b"20170606", b"20170707",
    b"20180914", b"20181225", b"20190320", b"20190415", b"20190618",
];

/// Serial number the charts are bound to; override with TEASI_DEVICE.
pub fn device() -> Vec<u8> {
    std::env::var("TEASI_DEVICE")
        .unwrap_or_else(|_| "2013021200000368".to_string())
        .into_bytes()
}

/// Today as "YYYYMMDD", the default version date of a freshly built chart.
/// Days to a calendar date after Howard Hinnant's civil_from_days.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs();
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{:04}{:02}{:02}", y, m, d)
}

pub fn global_key(device_id: &[u8]) -> Vec<u8> {
    let prefix = &device_id[..8.min(device_id.len())];
    if KNOWN_PREFIXES.iter().any(|p| p.as_slice() == prefix) {
        return STATIC_KEY.to_vec();
    }
    let digest = Md5::digest(prefix);
    format!("{:X}", digest).into_bytes()
}

/// Header MAC: generic with an empty device, otherwise bound to that device.
pub fn header_md5(d: &[u8], device: &[u8]) -> [u8; 16] {
    let mut h = Md5::new();
    h.update(&d[4..0x34]);
    h.update(SECRET);
    h.update(&d[0x44..0x444]);
    h.update(&d[d.len() - 0x400..]);
    h.update(device);
    h.finalize().into()
}

/// The firmware's country list: the header's country code at 0x54 is the index
/// into it (1..=338, `index < 0x153` is what `bikenav.exe` checks), with the
/// ISO 3166-1 alpha-3 code of the table that follows it in the binary and the
/// prefix of the file names.  Names and codes are read out of `bikenav.exe` at
/// 0x45bfa0; Denmark 4, Germany 7, Norway 12, Sweden 16 and United Kingdom 17
/// are confirmed against the files on the device.
///
/// The prefix is what the map list on the device shows (`FUN_000c5bec`): it
/// takes the file name in lower case, splits it at `_` and translates
/// `"country"` + `_<part>` for every part but the layer, with `usa` becoming
/// `_(united_states)` and `canada` `_(canada)`.  So `Czech_Republic` is shown
/// as the translation of `country_czech_republic`, while `CzechRepublic` comes
/// out as `#country_czechrepublic` (checked on the device 2026-11-02).  Every
/// prefix here has its key in `gpstuner.dat`; only Canada as a whole cannot
/// have one.  A few names point at the key the firmware has instead (Quebec,
/// Micronesia, the islands of the United States).
///
/// A code only passes the device's unlock table if that country is licensed for
/// it, which is not something this table can say.
pub static COUNTRIES: [(&str, u32, &str, &str); 338] = [
    ("Andorra", 1, "AND", "Andorra"),
    ("Austria", 2, "AUT", "Austria"),
    ("Belgium", 3, "BEL", "Belgium"),
    ("Denmark", 4, "DNK", "Denmark"),
    ("Finland", 5, "FIN", "Finland"),
    ("France", 6, "FRA", "France"),
    ("Germany", 7, "DEU", "Germany"),
    ("Ireland", 8, "IRL", "Ireland"),
    ("Italy", 9, "ITA", "Italy"),
    ("Luxembourg", 10, "LUX", "Luxembourg"),
    ("Netherlands", 11, "NLD", "Netherlands"),
    ("Norway", 12, "NOR", "Norway"),
    ("Portugal", 13, "PRT", "Portugal"),
    ("San Marino", 14, "SMR", "San_Marino"),
    ("Spain", 15, "ESP", "Spain"),
    ("Sweden", 16, "SWE", "Sweden"),
    ("United Kingdom", 17, "GBR", "United_Kingdom"),
    ("Hungary", 18, "HUN", "Hungary"),
    ("Switzerland", 19, "CHE", "Switzerland"),
    ("Monaco", 20, "MCO", "Monaco"),
    ("Liechtenstein", 21, "LIE", "Liechtenstein"),
    ("Gibraltar", 22, "GIB", "Gibraltar"),
    ("Vatican City", 23, "VAT", "Vatican_City"),
    ("Taiwan", 24, "TWN", "Taiwan"),
    ("Albania", 25, "ALB", "Albania"),
    ("Belarus", 26, "BLR", "Belarus"),
    ("Bosnia and Herzegovina", 27, "BIH", "Bosnia_and_Herzegovina"),
    ("Bulgaria", 28, "BGR", "Bulgaria"),
    ("Croatia", 29, "HRV", "Croatia"),
    ("Czech Republic", 30, "CZE", "Czech_Republic"),
    ("Estonia", 31, "EST", "Estonia"),
    ("FYR of Macedonia", 32, "MKD", "FYR_of_Macedonia"),
    ("Greece", 33, "GRC", "Greece"),
    ("Latvia", 34, "LVA", "Latvia"),
    ("Lithuania", 35, "LTU", "Lithuania"),
    ("Moldova", 36, "MDA", "Moldova"),
    ("Montenegro", 37, "MNE", "Montenegro"),
    ("Poland", 38, "POL", "Poland"),
    ("Romania", 39, "ROU", "Romania"),
    ("Serbia", 40, "SRB", "Serbia"),
    ("Slovakia", 41, "SVK", "Slovakia"),
    ("Slovenia", 42, "SVN", "Slovenia"),
    ("Turkey", 43, "TUR", "Turkey"),
    ("Ukraine", 44, "UKR", "Ukraine"),
    ("Australia", 45, "AUS", "Australia"),
    ("New Zealand", 46, "NZL", "New_Zealand"),
    ("Russia", 47, "RUS", "Russia"),
    ("United States", 48, "USA", "United_States"),
    ("Alabama (United States)", 49, "USA_AL", "Alabama_USA"),
    ("Alaska (United States)", 50, "USA_AK", "Alaska_USA"),
    ("American Samoa (United States)", 51, "USA_AS", "American_Samoa_USA"),
    ("Arizona (United States)", 52, "USA_AZ", "Arizona_USA"),
    ("Arkansas (United States)", 53, "USA_AR", "Arkansas_USA"),
    ("California (United States)", 54, "USA_CA", "California_USA"),
    ("Colorado (United States)", 55, "USA_CO", "Colorado_USA"),
    ("Connecticut (United States)", 56, "USA_CT", "Connecticut_USA"),
    ("Delaware (United States)", 57, "USA_DE", "Delaware_USA"),
    ("District Of Columbia (United States)", 58, "USA_DC", "District_Of_Columbia_USA"),
    ("Federated States Of Micronesia (United States)", 59, "USA_FM", "Federated_States_Of_Micronesia_USA"),
    ("Florida (United States)", 60, "USA_FL", "Florida_USA"),
    ("Georgia (United States)", 61, "USA_GA", "Georgia_USA"),
    ("Guam (United States)", 62, "USA_GU", "Guam_USA"),
    ("Hawaii (United States)", 63, "USA_HI", "Hawaii_USA"),
    ("Idaho (United States)", 64, "USA_ID", "Idaho_USA"),
    ("Illinois (United States)", 65, "USA_IL", "Illinois_USA"),
    ("Indiana (United States)", 66, "USA_IN", "Indiana_USA"),
    ("Iowa (United States)", 67, "USA_IA", "Iowa_USA"),
    ("Kansas (United States)", 68, "USA_KS", "Kansas_USA"),
    ("Kentucky (United States)", 69, "USA_KY", "Kentucky_USA"),
    ("Louisiana (United States)", 70, "USA_LA", "Louisiana_USA"),
    ("Maine (United States)", 71, "USA_ME", "Maine_USA"),
    ("Maryland (United States)", 72, "USA_MH", "Maryland_USA"),
    ("Marshall islands (United States)", 73, "USA_MD", "Marshall_Islands_USA"),
    ("Massachusetts (United States)", 74, "USA_MA", "Massachusetts_USA"),
    ("Michigan (United States)", 75, "USA_MI", "Michigan_USA"),
    ("Minnesota (United States)", 76, "USA_MN", "Minnesota_USA"),
    ("Mississippi (United States)", 77, "USA_MS", "Mississippi_USA"),
    ("Missouri (United States)", 78, "USA_MO", "Missouri_USA"),
    ("Montana (United States)", 79, "USA_MT", "Montana_USA"),
    ("Nebraska (United States)", 80, "USA_NE", "Nebraska_USA"),
    ("Nevada (United States)", 81, "USA_NV", "Nevada_USA"),
    ("New Hampshire (United States)", 82, "USA_NH", "New_Hampshire_USA"),
    ("New Jersey (United States)", 83, "USA_NJ", "New_Jersey_USA"),
    ("New Mexico (United States)", 84, "USA_NM", "New_Mexico_USA"),
    ("New York (United States)", 85, "USA_NY", "New_York_USA"),
    ("North Carolina (United States)", 86, "USA_NC", "North_Carolina_USA"),
    ("North Dakota (United States)", 87, "USA_ND", "North_Dakota_USA"),
    ("Northern Mariana Islands (United States)", 88, "USA_MP", "Northern_Mariana_Islands_USA"),
    ("Ohio (United States)", 89, "USA_OH", "Ohio_USA"),
    ("Oklahoma (United States)", 90, "USA_OK", "Oklahoma_USA"),
    ("Oregon (United States)", 91, "USA_OR", "Oregon_USA"),
    ("Palau (United States)", 92, "USA_PW", "Palau_USA"),
    ("Pennsylvania (United States)", 93, "USA_PA", "Pennsylvania_USA"),
    ("Puerto Rico (United States)", 94, "USA_PR", "Puerto_Rico_USA"),
    ("Rhode Island (United States)", 95, "USA_RI", "Rhode_Island_USA"),
    ("South Carolina (United States)", 96, "USA_SC", "South_Carolina_USA"),
    ("South Dakota (United States)", 97, "USA_SD", "South_Dakota_USA"),
    ("Tennessee (United States)", 98, "USA_TN", "Tennessee_USA"),
    ("Texas (United States)", 99, "USA_TX", "Texas_USA"),
    ("Utah (United States)", 100, "USA_UT", "Utah_USA"),
    ("Vermont (United States)", 101, "USA_VT", "Vermont_USA"),
    ("Virgin Islands (United States)", 102, "USA_VI", "Virgin_Islands_USA"),
    ("Virginia (United States)", 103, "USA_VA", "Virginia_USA"),
    ("Washington (United States)", 104, "USA_WA", "Washington_USA"),
    ("West Virginia (United States)", 105, "USA_WV", "West_Virginia_USA"),
    ("Wisconsin (United States)", 106, "USA_WI", "Wisconsin_USA"),
    ("Wyoming (United States)", 107, "USA_WY", "Wyoming_USA"),
    ("South Africa", 108, "ZAF", "South_Africa"),
    ("Malta", 109, "MLT", "Malta"),
    ("Iceland", 110, "ISL", "Iceland"),
    ("Botswana", 111, "BWA", "Botswana"),
    ("Swaziland", 112, "SWZ", "Swaziland"),
    ("Namibia", 113, "NAM", "Namibia"),
    ("Lesotho", 114, "LSO", "Lesotho"),
    ("Morocco", 115, "MAR", "Morocco"),
    ("Argentina", 116, "ARG", "Argentina"),
    ("Malaysia", 117, "MYS", "Malaysia"),
    ("Singapore", 118, "SGP", "Singapore"),
    ("South Korea", 119, "KOR", "South_Korea"),
    ("Brazil", 120, "BRA", "Brazil"),
    ("Mexico", 121, "MEX", "Mexico"),
    ("Chile", 122, "CHL", "Chile"),
    ("India", 123, "IND", "India"),
    ("Canada", 124, "CAN", "Canada"),
    ("Alberta (Canada)", 125, "CAN_AB", "Alberta_Canada"),
    ("British Columbia (Canada)", 126, "CAN_BC", "British_Columbia_Canada"),
    ("Manitoba (Canada)", 127, "CAN_MB", "Manitoba_Canada"),
    ("New Brunswick (Canada)", 128, "CAN_NB", "New_Brunswick_Canada"),
    ("Newfoundland And Labrador (Canada)", 129, "CAN_NL", "Newfoundland_And_Labrador_Canada"),
    ("Northwest Territories (Canada)", 130, "CAN_NT", "Northwest_Territories_Canada"),
    ("Nova Scotia (Canada)", 131, "CAN_NS", "Nova_Scotia_Canada"),
    ("Nunavut (Canada)", 132, "CAN_NU", "Nunavut_Canada"),
    ("Ontario (Canada)", 133, "CAN_ON", "Ontario_Canada"),
    ("Prince Edward Island (Canada)", 134, "CAN_PE", "Prince_Edward_Island_Canada"),
    ("Québec (Canada)", 135, "CAN_QC", "Quebec_Canada"),
    ("Saskatchewan (Canada)", 136, "CAN_SK", "Saskatchewan_Canada"),
    ("Yukon Territory (Canada)", 137, "CAN_YT", "Yukon_Territory_Canada"),
    ("Madagascar", 138, "MDG", "Madagascar"),
    ("Mauritius", 139, "MUS", "Mauritius"),
    ("Panama", 140, "PAN", "Panama"),
    ("Kosovo", 141, "XKS", "Kosovo"),
    ("China", 142, "CHN", "China"),
    ("Cyprus", 143, "CYP", "Cyprus"),
    ("United Arab Emirates", 144, "ARE", "United_Arab_Emirates"),
    ("Oman", 145, "OMN", "Oman"),
    ("Bahamas", 146, "BHS", "Bahamas"),
    ("Colombia", 147, "COL", "Colombia"),
    ("Peru", 148, "PER", "Peru"),
    ("Venezuela", 149, "VEN", "Venezuela"),
    ("Ecuador", 150, "ECU", "Ecuador"),
    ("Bolivia", 151, "BOL", "Bolivia"),
    ("Paraguay", 152, "PRY", "Paraguay"),
    ("Uruguay", 153, "URY", "Uruguay"),
    ("Guyana", 154, "GUY", "Guyana"),
    ("Suriname", 155, "SUR", "Suriname"),
    ("French Guiana", 156, "GUF", "French_Guiana"),
    ("Falkland Islands", 157, "FLK", "Falkland_Islands"),
    ("Japan", 158, "JPN", "Japan"),
    ("Israel", 159, "ISR", "Israel"),
    ("Indonesia", 160, "IDN", "Indonesia"),
    ("Thailand", 161, "THA", "Thailand"),
    ("Philippines", 162, "PHL", "Philippines"),
    ("Vietnam", 163, "VNM", "Vietnam"),
    ("Bahrain", 164, "BHR", "Bahrain"),
    ("Iran", 165, "IRN", "Iran"),
    ("Lebanon", 166, "LBN", "Lebanon"),
    ("Saudi Arabia", 167, "SAU", "Saudi_Arabia"),
    ("Qatar", 168, "QAT", "Qatar"),
    ("Armenia", 169, "ARM", "Armenia"),
    ("Azerbaijan", 170, "AZE", "Azerbaijan"),
    ("Georgia", 171, "GEO", "Georgia"),
    ("Kazakhstan", 172, "KAZ", "Kazakhstan"),
    ("Uzbekistan", 173, "UZB", "Uzbekistan"),
    ("Algeria", 174, "DZA", "Algeria"),
    ("Egypt", 175, "EGY", "Egypt"),
    ("Libya", 176, "LBY", "Libya"),
    ("Tunisia", 177, "TUN", "Tunisia"),
    ("Dominican Republic", 178, "DOM", "Dominican_Republic"),
    ("Aruba", 179, "ABW", "Aruba"),
    ("Afghanistan", 180, "AFG", "Afghanistan"),
    ("Angola", 181, "AGO", "Angola"),
    ("Anguilla", 182, "AIA", "Anguilla"),
    ("Aland Islands", 183, "ALA", "Aland_Islands"),
    ("Netherlands Antilles", 184, "ANT", "Netherlands_Antilles"),
    ("American Samoa", 185, "ASM", "American_Samoa_USA"),
    ("Antarctica", 186, "ATA", "Antarctica"),
    ("French Southern Territories", 187, "ATF", "French_Southern_Territories"),
    ("Antigua and Barbuda", 188, "ATG", "Antigua_and_Barbuda"),
    ("Burundi", 189, "BDI", "Burundi"),
    ("Benin", 190, "BEN", "Benin"),
    ("Burkina Faso", 191, "BFA", "Burkina_Faso"),
    ("Bangladesh", 192, "BGD", "Bangladesh"),
    ("Saint Barthelemy", 193, "BLM", "Saint_Barthelemy"),
    ("Belize", 194, "BLZ", "Belize"),
    ("Bermuda", 195, "BMU", "Bermuda"),
    ("Barbados", 196, "BRB", "Barbados"),
    ("Brunei Darussalam", 197, "BRN", "Brunei_Darussalam"),
    ("Bhutan", 198, "BTN", "Bhutan"),
    ("Bouvet Island", 199, "BVT", "Bouvet_Island"),
    ("Central African Republic", 200, "CAF", "Central_African_Republic"),
    ("Hong Kong", 201, "HKG", "Hong_Kong"),
    ("Cocos (Keeling) Islands", 202, "CCK", "Cocos_(Keeling)_Islands"),
    ("Ivory Coast", 203, "CIV", "Ivory_Coast"),
    ("Cameroon", 204, "CMR", "Cameroon"),
    ("Democratic Republic of Congo", 205, "COD", "Democratic_Republic_of_Congo"),
    ("Congo", 206, "COG", "Congo"),
    ("Cook Islands", 207, "COK", "Cook_Islands"),
    ("Comoros", 208, "COM", "Comoros"),
    ("Cape Verde", 209, "CPV", "Cape_Verde"),
    ("Costa Rica", 210, "CRI", "Costa_Rica"),
    ("Cuba", 211, "CUB", "Cuba"),
    ("Christmas Island", 212, "CXR", "Christmas_Island"),
    ("Cayman Islands", 213, "CYM", "Cayman_Islands"),
    ("Djibouti", 214, "DJI", "Djibouti"),
    ("Dominica", 215, "DMA", "Dominica"),
    ("Eritrea", 216, "ERI", "Eritrea"),
    ("Western Sahara", 217, "ESH", "Western_Sahara"),
    ("Ethiopia", 218, "ETH", "Ethiopia"),
    ("Fiji", 219, "FJI", "Fiji"),
    ("Faroe Islands", 220, "FRO", "Faroe_Islands"),
    ("Micronesia, Federated States of", 221, "FSM", "Micronesia"),
    ("Gabon", 222, "GAB", "Gabon"),
    ("Guernsey", 223, "GGY", "Guernsey"),
    ("Ghana", 224, "GHA", "Ghana"),
    ("Guinea", 225, "GIN", "Guinea"),
    ("French Guadeloupe", 226, "GLP", "French_Guadeloupe"),
    ("Gambia", 227, "GMB", "Gambia"),
    ("Guinea-Bissau", 228, "GNB", "Guinea-Bissau"),
    ("Equatorial Guinea", 229, "GNQ", "Equatorial_Guinea"),
    ("Grenada", 230, "GRD", "Grenada"),
    ("Greenland", 231, "GRL", "Greenland"),
    ("Guatemala", 232, "GTM", "Guatemala"),
    ("Guam", 233, "GUM", "Guam_USA"),
    ("Heard Island and McDonald Islands", 234, "HMD", "Heard_Island_and_McDonald_Islands"),
    ("Honduras", 235, "HND", "Honduras"),
    ("Haiti", 236, "HTI", "Haiti"),
    ("Isle Of Man", 237, "IMN", "Isle_Of_Man"),
    ("British Indian Ocean Territory", 238, "IOT", "British_Indian_Ocean_Territory"),
    ("Iraq", 239, "IRQ", "Iraq"),
    ("Jamaica", 240, "JAM", "Jamaica"),
    ("Jersey", 241, "JEY", "Jersey"),
    ("Jordan", 242, "JOR", "Jordan"),
    ("Kenya", 243, "KEN", "Kenya"),
    ("Kyrgyzstan", 244, "KGZ", "Kyrgyzstan"),
    ("Cambodia", 245, "KHM", "Cambodia"),
    ("Kiribati", 246, "KIR", "Kiribati"),
    ("Saint Kitts and Nevis", 247, "KNA", "Saint_Kitts_and_Nevis"),
    ("Kuwait", 248, "KWT", "Kuwait"),
    ("Lao People s Democratic Republic", 249, "LAO", "Lao_People_s_Democratic_Republic"),
    ("Liberia", 250, "LBR", "Liberia"),
    ("Saint Lucia", 251, "LCA", "Saint_Lucia"),
    ("Sri Lanka", 252, "LKA", "Sri_Lanka"),
    ("Macau", 253, "MAC", "Macau"),
    ("Saint Martin", 254, "MAF", "Saint_Martin"),
    ("Maldives", 255, "MDV", "Maldives"),
    ("Marshall Islands", 256, "MHL", "Marshall_Islands_USA"),
    ("Mali", 257, "MLI", "Mali"),
    ("Myanmar", 258, "MMR", "Myanmar"),
    ("Mongolia", 259, "MNG", "Mongolia"),
    ("Northern Mariana Islands", 260, "MNP", "Northern_Mariana_Islands_USA"),
    ("Mozambique", 261, "MOZ", "Mozambique"),
    ("Mauritania", 262, "MRT", "Mauritania"),
    ("Montserrat", 263, "MSR", "Montserrat"),
    ("Martinique", 264, "MTQ", "Martinique"),
    ("Malawi", 265, "MWI", "Malawi"),
    ("Mayotte", 266, "MYT", "Mayotte"),
    ("New Caledonia", 267, "NCL", "New_Caledonia"),
    ("Niger", 268, "NER", "Niger"),
    ("Norfolk Island", 269, "NFK", "Norfolk_Island"),
    ("Nigeria", 270, "NGA", "Nigeria"),
    ("Nicaragua", 271, "NIC", "Nicaragua"),
    ("Niue", 272, "NIU", "Niue"),
    ("Nepal", 273, "NPL", "Nepal"),
    ("Nauru", 274, "NRU", "Nauru"),
    ("Pakistan", 275, "PAK", "Pakistan"),
    ("Pitcairn", 276, "PCN", "Pitcairn"),
    ("Palau", 277, "PLW", "Palau_USA"),
    ("Papua New Guinea", 278, "PNG", "Papua_New_Guinea"),
    ("Puerto Rico", 279, "PRI", "Puerto_Rico_USA"),
    ("North Korea", 280, "PRK", "North_Korea"),
    ("Palestinian Territory", 281, "PSE", "Palestinian_Territory"),
    ("French Polynesia", 282, "PYF", "French_Polynesia"),
    ("Reunion", 283, "REU", "Reunion"),
    ("Rwanda", 284, "RWA", "Rwanda"),
    ("Sudan", 285, "SDN", "Sudan"),
    ("South Sudan", 286, "SSD", "South_Sudan"),
    ("Senegal", 287, "SEN", "Senegal"),
    ("South Georgia and the South Sandwich Islands", 288, "SGS", "South_Georgia_and_the_South_Sandwich_Islands"),
    ("Saint Helena, Ascension and Tristan da Cunha", 289, "SHN", "Saint_Helena,_Ascension_and_Tristan_da_Cunha"),
    ("Svalbard", 290, "SJM", "Svalbard"),
    ("Solomon Islands", 291, "SLB", "Solomon_Islands"),
    ("Sierra Leone", 292, "SLE", "Sierra_Leone"),
    ("El Salvador", 293, "SLV", "El_Salvador"),
    ("Somalia", 294, "SOM", "Somalia"),
    ("Saint Pierre And Miquelon", 295, "SPM", "Saint_Pierre_And_Miquelon"),
    ("Sao Tome and Principe", 296, "STP", "Sao_Tome_and_Principe"),
    ("Seychlles", 297, "SYC", "Seychlles"),
    ("Syria", 298, "SYR", "Syria"),
    ("Turks and Caicos Islands", 299, "TCA", "Turks_and_Caicos_Islands"),
    ("Chad", 300, "TCD", "Chad"),
    ("Togo", 301, "TGO", "Togo"),
    ("Tajikistan", 302, "TJK", "Tajikistan"),
    ("Tokelau", 303, "TKL", "Tokelau"),
    ("Turkmenistan", 304, "TKM", "Turkmenistan"),
    ("East Timor", 305, "TLS", "East_Timor"),
    ("Tonga", 306, "TON", "Tonga"),
    ("Trinidad and Tobago", 307, "TTO", "Trinidad_and_Tobago"),
    ("Tuvalu", 308, "TUV", "Tuvalu"),
    ("Tanzania", 309, "TZA", "Tanzania"),
    ("Uganda", 310, "UGA", "Uganda"),
    ("United States Minor Outlying Islands", 311, "UMI", "United_States_Minor_Outlying_Islands"),
    ("Saint Vincent and the Grenadines", 312, "VCT", "Saint_Vincent_and_the_Grenadines"),
    ("British Virgin Islands", 313, "VGB", "British_Virgin_Islands"),
    ("Virgin Islands, U.S.", 314, "VIR", "Virgin_Islands_USA"),
    ("Vanuatu", 315, "VUT", "Vanuatu"),
    ("Wallis and Futuna", 316, "WLF", "Wallis_and_Futuna"),
    ("Samoa", 317, "WSM", "Samoa"),
    ("Yemen", 318, "YEM", "Yemen"),
    ("Zambia", 319, "ZMB", "Zambia"),
    ("Zimbabwe", 320, "ZWE", "Zimbabwe"),
    ("England", 321, "GBR_EN", "England"),
    ("Northern Ireland", 322, "GBR_NI", "Northern_Ireland"),
    ("Wales", 323, "GBR_WL", "Wales"),
    ("Scotland", 324, "GBR_SC", "Scotland"),
    ("Cyprus Un Neutral Zone", 325, "CYP_NZ", "Cyprus_Un_Neutral_Zone"),
    ("Channel Islands", 326, "CHANNEL_ISLANDS", "Channel_Islands"),
    ("New South Wales (Australia)", 327, "AUS_NSW", "New_South_Wales_(Australia)"),
    ("Northern Territory (Australia)", 328, "AUS_NT", "Northern_Territory_(Australia)"),
    ("Victoria (Australia)", 329, "AUS_VIC", "Victoria_(Australia)"),
    ("South Australia (Australia)", 330, "AUS_SA", "South_Australia_(Australia)"),
    ("Tasmania (Australia)", 331, "AUS_TAS", "Tasmania_(Australia)"),
    ("Queensland (Australia)", 332, "AUS_QLD", "Queensland_(Australia)"),
    ("Western Australia (Australia)", 333, "AUS_WA", "Western_Australia_(Australia)"),
    ("Australian Capital Territory (Australia)", 334, "AUS_ACT", "Australian_Capital_Territory_(Australia)"),
    ("Other Territories (Australia)", 335, "AUS_OT", "Other_Territories_(Australia)"),
    ("Gaza Strip", 336, "GZS", "Gaza_Strip"),
    ("Curacao", 337, "CUR", "Curacao"),
    ("West Bank", 338, "WBK", "West_Bank"),
];

/// One country of `COUNTRIES`, by name, by ISO code or by its number.  Spaces,
/// underscores and case are ignored, so `--country="san marino"`, `SMR` and
/// `14` are the same thing.  A number that is not in the table is passed
/// through, because only the device's unlock table decides what it loads --
/// the name then has to be given separately.
pub fn country(s: &str) -> Result<(u32, Option<&'static str>, Option<&'static str>)> {
    let norm = |t: &str| t.to_lowercase().replace(['_', '-', ' ', '(', ')'], "");
    let q = norm(s);
    // "New South Wales" and "Quebec" also find "New South Wales (Australia)"
    // and "Quebec_Canada" -- the accent-free prefix is what makes the latter work
    let head = |n: &str| {
        let n = n.trim_end_matches("_USA").trim_end_matches("_Canada");
        norm(n.split('(').next().unwrap_or(n))
    };
    // a full match first: "Georgia" is the country, "Georgia (United States)"
    // only matches it by the part before the bracket
    let full = COUNTRIES.iter().find(|(n, _, i, f)| [norm(n), norm(i), norm(f)].contains(&q));
    let part = || COUNTRIES.iter().find(|(n, _, _, f)| [head(n), head(f)].contains(&q));
    if let Some(&(name, code, _, file)) = full.or_else(part) {
        return Ok((code, Some(name), Some(file)));
    }
    let code = s
        .parse()
        .map_err(|_| anyhow::anyhow!("unknown country {:?}, see chart::COUNTRIES", s))?;
    match COUNTRIES.iter().find(|(_, c, _, _)| *c == code) {
        Some(&(name, _, _, file)) => Ok((code, Some(name), Some(file))),
        None => Ok((code, None, None)),
    }
}

/// Who a freshly built file is signed for.  The serial always picks the record
/// key (`global_key`), `bind` decides what the header MAC covers:
///
/// - **bound** names the device, like every original file.
/// - **generic** leaves it out, and the firmware binds the file to whatever
///   device opens it first by rewriting salt and MAC (`FUN_001061bc`, see
///   docs/CHART_FILES.md 2).  The `packages.xml` checksum survives that,
///   because it starts behind both.
///
/// A generic file therefore runs on every device whose serial starts with the
/// same eight digits -- that is what the record key is derived from.
pub struct Signer {
    pub device: Vec<u8>,
    pub bind: bool,
}

impl Signer {
    /// Bound to `TEASI_DEVICE`, or to the serial above.
    pub fn bound() -> Signer {
        Signer { device: device(), bind: true }
    }

    /// Signed for no device in particular.
    pub fn generic() -> Signer {
        Signer { device: device(), bind: false }
    }

    pub fn key(&self) -> Vec<u8> {
        global_key(&self.device)
    }

    /// The header MAC of a finished file.  Both the MAC and the firmware's own
    /// check read 1 KB from 0x44 and the last KB, so anything shorter than that
    /// cannot be a chart file -- which is what a layer with no records at all
    /// comes out as.
    pub fn mac(&self, d: &[u8]) -> Result<[u8; 16]> {
        ensure!(
            d.len() >= 0x844,
            "{} bytes is too short for a chart file: the checksum covers 1 KB from 0x44 \
             and the last KB, so there is nothing to sign",
            d.len()
        );
        Ok(header_md5(d, if self.bind { &self.device } else { b"" }))
    }
}

/// The checksum `BikeNav/packages.xml` carries for a map file: 1 KB from 0x44
/// (behind the salt and the MAC) plus the last KB, see docs/CHART_FILES.md 5.4.
pub fn package_md5(d: &[u8]) -> md5::digest::Output<Md5> {
    let mut h = Md5::new();
    h.update(&d[0x44..0x444]);
    h.update(&d[d.len() - 0x400..]);
    h.finalize()
}

pub fn u32_at(d: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(d[off..off + 4].try_into().unwrap())
}

pub fn u16_at(d: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(d[off..off + 2].try_into().unwrap())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub x: u16,
    pub y: u16,
    pub start: usize,
    pub end: usize,
}

/// Directory at 0x78; a tile ends where the next one starts.
pub fn tiles(d: &[u8]) -> Vec<Tile> {
    let n = u32_at(d, 0x74) as usize;
    let dir: Vec<(u16, u16, usize)> = (0..n)
        .map(|i| {
            let o = 0x78 + 8 * i;
            (u16_at(d, o), u16_at(d, o + 2), u32_at(d, o + 4) as usize)
        })
        .collect();
    dir.iter()
        .enumerate()
        .map(|(i, &(x, y, off))| Tile {
            x,
            y,
            start: off,
            end: if i + 1 < n { dir[i + 1].2 } else { d.len() },
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub struct Rec {
    pub rel: usize,
    pub len: usize,
    pub pltx: usize,
}

/// The records of one tile: a gapless chain of [u32 len][u32 pltx][payload]
/// from +0x159C to the end of the tile.
pub fn records(d: &[u8], start: usize, end: usize) -> Vec<Rec> {
    let mut out = Vec::new();
    let mut rel = HEAD;
    while start + rel + 8 <= end {
        let len = u32_at(d, start + rel) as usize;
        let pltx = u32_at(d, start + rel + 4) as usize;
        if len == 0 || start + rel + 8 + len > end {
            break;
        }
        out.push(Rec { rel, len, pltx });
        rel += 8 + len;
    }
    out
}

/// PC1 then LZMA.  osm's slot area B (routing graph) is stored LZMA-only;
/// those records are recognised by the fallback.
pub fn decode_record(payload: &[u8], pltx: usize, rk: &[u8]) -> Result<Vec<u8>> {
    if let Ok(out) = lzma::decompress(&pc1::decrypt_payload(payload, rk), pltx) {
        return Ok(out);
    }
    match lzma::decompress(payload, pltx) {
        Ok(out) => Ok(out),
        Err(e) => bail!("record does not decode ({} plaintext bytes): {}", pltx, e),
    }
}

/// The tile's record key: 32 bytes at +0x157C, PC1 encrypted with the global key.
pub fn record_key(d: &[u8], tile_start: usize, key: &[u8]) -> Vec<u8> {
    pc1::decrypt_blob(&d[tile_start + BLOB..tile_start + HEAD], key)
}

pub struct Chart {
    pub data: Vec<u8>,
    pub key: Vec<u8>,
}

impl Chart {
    pub fn open(path: &str) -> Result<Chart> {
        let data = std::fs::read(path)?;
        if data.len() < 0x844 || u32_at(&data, 0) != MAGIC {
            bail!("{}: not a chart file (magic 0x1B62)", path);
        }
        Ok(Chart { key: global_key(&device()), data })
    }

    pub fn date(&self) -> &[u8] {
        &self.data[0x44..0x4C]
    }

    pub fn typ(&self) -> u32 {
        u32_at(&self.data, 0x4C)
    }

    pub fn layer(&self) -> u32 {
        u32_at(&self.data, 0x50)
    }

    pub fn country(&self) -> u32 {
        u32_at(&self.data, 0x54)
    }

    pub fn extra(&self) -> u32 {
        u32_at(&self.data, 0x70)
    }

    pub fn tiles(&self) -> Vec<Tile> {
        tiles(&self.data)
    }

    pub fn bound(&self, device: &[u8]) -> bool {
        header_md5(&self.data, device) == self.data[0x34..0x44]
    }

    pub fn generic(&self) -> bool {
        header_md5(&self.data, b"") == self.data[0x34..0x44]
    }

    /// Plaintext of one record of a tile.
    pub fn record(&self, tile: &Tile, rec: &Rec) -> Result<Vec<u8>> {
        let rk = record_key(&self.data, tile.start, &self.key);
        let from = tile.start + rec.rel + 8;
        decode_record(&self.data[from..from + rec.len], rec.pltx, &rk)
    }
}
