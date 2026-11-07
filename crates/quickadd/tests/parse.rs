//! Behaviour of `parse` as a caller sees it. "Now" is Friday 6 Nov 2026, 10:00, unless a test
//! says otherwise.

use app_quickadd::{parse, Date, Due, Known, Now, QuickAdd, Repeat, Time, TokenKind, Weekday};

fn date(y: i32, m: u8, d: u8) -> Date {
    Date::new(y, m, d).unwrap()
}

fn time(h: u8, m: u8) -> Time {
    Time::new(h, m).unwrap()
}

fn now_at(y: i32, mo: u8, d: u8, h: u8, mi: u8) -> Now {
    Now { date: date(y, mo, d), time: time(h, mi) }
}

const FRI: (i32, u8, u8) = (2026, 11, 6);

fn known() -> Known {
    let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
    Known {
        lists: v(&["Belanja", "Tagihan", "Sprint", "Invoice", "Inbox", "Admin", "Archive"]),
        folders: v(&["Family", "Family/Rumah", "Work", "Work/Client A", "Work/Client B", "Work/Admin"]),
        people: v(&["Dimas", "Ibu", "Ayah", "Dewi Lestari"]),
    }
}

fn p(input: &str) -> QuickAdd {
    parse(input, now_at(FRI.0, FRI.1, FRI.2, 10, 0), &known())
}

fn due(q: &QuickAdd) -> (Date, Option<Time>) {
    let d = q.due.expect("expected a due date");
    (d.date, d.time)
}

fn kinds(q: &QuickAdd) -> Vec<TokenKind> {
    q.tokens.iter().map(|t| t.kind).collect()
}

fn token_texts(q: &QuickAdd) -> Vec<&str> {
    q.tokens.iter().map(|t| t.text.as_str()).collect()
}

// ---- The spec's own examples -------------------------------------------------------------

#[test]
fn spec_beli_galon_besok_9am_belanja() {
    let input = "beli galon besok 9am #belanja";
    let q = p(input);
    assert_eq!(q.title, "beli galon");
    assert_eq!(q.due, Some(Due { date: date(2026, 11, 7), time: Some(time(9, 0)) }));
    assert_eq!(q.list.as_deref(), Some("Belanja"));
    assert_eq!(q.repeat, Repeat::None);
    assert_eq!(kinds(&q), [TokenKind::Date, TokenKind::Time, TokenKind::List]);
    assert_eq!(token_texts(&q), ["besok", "9am", "#belanja"]);
    for t in &q.tokens {
        assert_eq!(&input[t.range.clone()], t.text);
    }
    assert_eq!(q.tokens[0].range, 11..16);
}

#[test]
fn spec_rapat_lusa_jam_3_dimas() {
    let q = p("rapat lusa jam 3 @dimas");
    assert_eq!(q.title, "rapat");
    assert_eq!(due(&q), (date(2026, 11, 8), Some(time(15, 0))));
    assert_eq!(q.assignee.as_deref(), Some("Dimas"));
    assert_eq!(token_texts(&q), ["lusa", "jam 3", "@dimas"]);
    assert_eq!(kinds(&q), [TokenKind::Date, TokenKind::Time, TokenKind::Person]);
}

#[test]
fn spec_kirim_invoice_jumat_work_client_a() {
    let q = p("Kirim invoice jumat /work/client-a");
    assert_eq!(q.title, "Kirim invoice");
    // Today is Friday, so "jumat" is next week's.
    assert_eq!(due(&q), (date(2026, 11, 13), None));
    assert_eq!(q.folder_path.as_deref(), Some("Work/Client A"));
    assert_eq!(q.list, None);
    assert_eq!(kinds(&q), [TokenKind::Date, TokenKind::Folder]);
}

#[test]
fn spec_beli_galon_besok_9am_without_list() {
    let q = p("beli galon besok 9am");
    assert_eq!(q.title, "beli galon");
    assert_eq!(due(&q), (date(2026, 11, 7), Some(time(9, 0))));
    assert_eq!(q.list, None);
}

// ---- Relative days ----------------------------------------------------------------------

#[test]
fn hari_ini_and_today_are_today() {
    assert_eq!(due(&p("bayar listrik hari ini")), (date(2026, 11, 6), None));
    assert_eq!(due(&p("pay rent today")), (date(2026, 11, 6), None));
    assert_eq!(p("bayar listrik hari ini").title, "bayar listrik");
}

#[test]
fn besok_and_tomorrow_are_the_next_day() {
    assert_eq!(due(&p("besok cuci mobil")), (date(2026, 11, 7), None));
    assert_eq!(due(&p("Tomorrow call mom")).0, date(2026, 11, 7));
    assert_eq!(p("Tomorrow call mom").title, "call mom");
}

#[test]
fn lusa_and_the_day_after_tomorrow_are_two_days_ahead() {
    assert_eq!(due(&p("servis motor lusa")).0, date(2026, 11, 8));
    let q = p("dentist the day after tomorrow");
    assert_eq!(due(&q).0, date(2026, 11, 8));
    assert_eq!(q.title, "dentist");
    assert_eq!(token_texts(&q), ["the day after tomorrow"]);
    assert_eq!(due(&p("dentist day after tomorrow")).0, date(2026, 11, 8));
}

#[test]
fn tomorrow_rolls_over_the_year() {
    let q = parse("tutup buku besok", now_at(2026, 12, 31, 9, 0), &known());
    assert_eq!(due(&q).0, date(2027, 1, 1));
}

// ---- Weekdays ---------------------------------------------------------------------------

#[test]
fn indonesian_weekdays_mean_the_next_occurrence() {
    assert_eq!(due(&p("senin")).0, date(2026, 11, 9));
    assert_eq!(due(&p("selasa")).0, date(2026, 11, 10));
    assert_eq!(due(&p("rabu")).0, date(2026, 11, 11));
    assert_eq!(due(&p("kamis")).0, date(2026, 11, 12));
    assert_eq!(due(&p("jumat")).0, date(2026, 11, 13));
    assert_eq!(due(&p("sabtu")).0, date(2026, 11, 7));
    assert_eq!(due(&p("minggu")).0, date(2026, 11, 8));
    assert_eq!(due(&p("ahad")).0, date(2026, 11, 8));
}

#[test]
fn english_weekdays_and_short_forms() {
    assert_eq!(due(&p("monday")).0, date(2026, 11, 9));
    assert_eq!(due(&p("tue")).0, date(2026, 11, 10));
    assert_eq!(due(&p("wed")).0, date(2026, 11, 11));
    assert_eq!(due(&p("thurs")).0, date(2026, 11, 12));
    assert_eq!(due(&p("fri")).0, date(2026, 11, 13));
    assert_eq!(due(&p("sat")).0, date(2026, 11, 7));
    assert_eq!(due(&p("Sunday")).0, date(2026, 11, 8));
}

#[test]
fn indonesian_short_weekday_forms() {
    assert_eq!(due(&p("rab")).0, date(2026, 11, 11));
    assert_eq!(due(&p("kam")).0, date(2026, 11, 12));
    assert_eq!(due(&p("jum'at")).0, date(2026, 11, 13));
    assert_eq!(due(&p("sab")).0, date(2026, 11, 7));
}

#[test]
fn weekdays_are_case_insensitive() {
    let q = p("Laporan JUMAT");
    assert_eq!(due(&q).0, date(2026, 11, 13));
    assert_eq!(q.title, "Laporan");
    assert_eq!(token_texts(&q), ["JUMAT"]);
}

#[test]
fn hari_before_a_weekday_is_part_of_the_date() {
    let q = p("arisan hari sabtu");
    assert_eq!(due(&q).0, date(2026, 11, 7));
    assert_eq!(q.title, "arisan");
    assert_eq!(token_texts(&q), ["hari sabtu"]);
}

#[test]
fn senin_depan_and_next_monday_are_in_next_week() {
    assert_eq!(due(&p("senin depan")).0, date(2026, 11, 9));
    assert_eq!(due(&p("next monday")).0, date(2026, 11, 9));
    // Saturday tomorrow is this week's; "sabtu depan" is the week after.
    assert_eq!(due(&p("sabtu depan")).0, date(2026, 11, 14));
    assert_eq!(due(&p("next saturday")).0, date(2026, 11, 14));
    let q = p("presentasi hari kamis depan");
    assert_eq!(due(&q).0, date(2026, 11, 12));
    assert_eq!(q.title, "presentasi");
}

#[test]
fn minggu_depan_and_next_week_are_monday_of_next_week() {
    let q = p("review kontrak minggu depan");
    assert_eq!(due(&q).0, date(2026, 11, 9));
    assert_eq!(q.title, "review kontrak");
    assert_eq!(token_texts(&q), ["minggu depan"]);
    assert_eq!(due(&p("plan trip next week")).0, date(2026, 11, 9));
    assert_eq!(due(&p("pekan depan")).0, date(2026, 11, 9));
}

#[test]
fn next_week_from_a_sunday_is_the_next_day() {
    let sunday = now_at(2026, 11, 8, 9, 0);
    assert_eq!(due(&parse("x minggu depan", sunday, &known())).0, date(2026, 11, 9));
    assert_eq!(due(&parse("x senin", sunday, &known())).0, date(2026, 11, 9));
}

// ---- Calendar dates ---------------------------------------------------------------------

#[test]
fn tgl_picks_the_next_matching_day_of_month() {
    assert_eq!(due(&p("bayar kos tgl 15")).0, date(2026, 11, 15));
    assert_eq!(due(&p("tanggal 6")).0, date(2026, 11, 6));
    assert_eq!(due(&p("tgl 3")).0, date(2026, 12, 3));
    // November has 30 days, so the 31st is December's.
    assert_eq!(due(&p("tgl 31")).0, date(2026, 12, 31));
    assert_eq!(p("bayar kos tgl 15").title, "bayar kos");
}

#[test]
fn day_then_month_names() {
    assert_eq!(due(&p("ulang tahun 15 nov")).0, date(2026, 11, 15));
    assert_eq!(due(&p("17 agustus")).0, date(2027, 8, 17));
    assert_eq!(due(&p("1 jan")).0, date(2027, 1, 1));
    assert_eq!(due(&p("25 Desember")).0, date(2026, 12, 25));
    assert_eq!(due(&p("tgl 20 mei")).0, date(2027, 5, 20));
    assert_eq!(token_texts(&p("ulang tahun 15 nov")), ["15 nov"]);
}

#[test]
fn month_then_day_in_english() {
    let q = p("submit report nov 15");
    assert_eq!(due(&q).0, date(2026, 11, 15));
    assert_eq!(q.title, "submit report");
    assert_eq!(due(&p("december 1")).0, date(2026, 12, 1));
    assert_eq!(due(&p("march 3")).0, date(2027, 3, 3));
}

#[test]
fn day_slash_month() {
    assert_eq!(due(&p("perpanjang SIM 15/11")).0, date(2026, 11, 15));
    // 5 Nov has passed this year.
    assert_eq!(due(&p("5/11")).0, date(2027, 11, 5));
    assert_eq!(due(&p("1/3")).0, date(2027, 3, 1));
}

#[test]
fn impossible_dates_are_left_in_the_title() {
    let q = p("cek 31/11 lagi");
    assert_eq!(q.due, None);
    assert_eq!(q.title, "cek 31/11 lagi");
    assert_eq!(p("31 nov").due, None);
    assert_eq!(p("15/13").due, None);
}

// ---- Times ------------------------------------------------------------------------------

#[test]
fn jam_with_a_small_hour_means_afternoon() {
    let q = p("telpon jam 3");
    assert_eq!(due(&q), (date(2026, 11, 6), Some(time(15, 0))));
    assert_eq!(q.title, "telpon");
    assert_eq!(due(&p("jam 6")).1, Some(time(18, 0)));
    assert_eq!(due(&p("jam 1")).1, Some(time(13, 0)));
}

#[test]
fn jam_with_larger_hours_is_read_as_written() {
    assert_eq!(due(&p("jam 7")), (date(2026, 11, 7), Some(time(7, 0))));
    assert_eq!(due(&p("jam 15")), (date(2026, 11, 6), Some(time(15, 0))));
    assert_eq!(due(&p("jam 12")).1, Some(time(12, 0)));
}

#[test]
fn jam_with_day_parts() {
    assert_eq!(due(&p("jemput jam 3 sore")).1, Some(time(15, 0)));
    assert_eq!(due(&p("jam 7 pagi")).1, Some(time(7, 0)));
    assert_eq!(due(&p("jam 8 malam")).1, Some(time(20, 0)));
    assert_eq!(due(&p("jam 1 siang")).1, Some(time(13, 0)));
    assert_eq!(due(&p("jam 11 siang")).1, Some(time(11, 0)));
    assert_eq!(due(&p("jam 12 malam")).1, Some(time(0, 0)));
    let q = p("jemput jam 3 sore");
    assert_eq!(q.title, "jemput");
    assert_eq!(token_texts(&q), ["jam 3 sore"]);
}

#[test]
fn pukul_works_like_jam() {
    assert_eq!(due(&p("rapat pukul 10")), (date(2026, 11, 7), Some(time(10, 0))));
    assert_eq!(due(&p("pukul 2 siang")).1, Some(time(14, 0)));
    assert_eq!(due(&p("pukul 10.30")).1, Some(time(10, 30)));
}

#[test]
fn jam_with_minutes() {
    assert_eq!(due(&p("jam 3.30")).1, Some(time(15, 30)));
    assert_eq!(due(&p("jam 9:15")).1, Some(time(9, 15)));
    assert_eq!(due(&p("jam 9am")).1, Some(time(9, 0)));
}

#[test]
fn am_pm_glued_or_spaced() {
    assert_eq!(due(&p("standup 9am")).1, Some(time(9, 0)));
    let q = p("gym 9 pm");
    assert_eq!(due(&q), (date(2026, 11, 6), Some(time(21, 0))));
    assert_eq!(q.title, "gym");
    assert_eq!(token_texts(&q), ["9 pm"]);
    assert_eq!(due(&p("call 7:30pm")).1, Some(time(19, 30)));
    assert_eq!(due(&p("x 12am")).1, Some(time(0, 0)));
    assert_eq!(due(&p("x 12pm")).1, Some(time(12, 0)));
    assert_eq!(due(&p("x 9 AM")).1, Some(time(9, 0)));
}

#[test]
fn clock_with_colon_or_dot() {
    assert_eq!(due(&p("meeting 15:30")), (date(2026, 11, 6), Some(time(15, 30))));
    assert_eq!(due(&p("rapat 15.30")), (date(2026, 11, 6), Some(time(15, 30))));
    assert_eq!(due(&p("rapat 08.00")), (date(2026, 11, 7), Some(time(8, 0))));
    assert_eq!(due(&p("rapat 7.30 malam")).1, Some(time(19, 30)));
}

#[test]
fn a_time_without_a_date_is_today_if_still_ahead_else_tomorrow() {
    let ten = now_at(2026, 11, 6, 10, 0);
    let k = known();
    assert_eq!(due(&parse("x 10:01", ten, &k)).0, date(2026, 11, 6));
    assert_eq!(due(&parse("x 10:00", ten, &k)).0, date(2026, 11, 7));
    assert_eq!(due(&parse("x 9:59", ten, &k)).0, date(2026, 11, 7));
}

#[test]
fn date_and_time_together() {
    let q = p("besok jam 7 pagi berangkat");
    assert_eq!(due(&q), (date(2026, 11, 7), Some(time(7, 0))));
    assert_eq!(q.title, "berangkat");
    // A date given explicitly keeps a time that has already passed today.
    assert_eq!(due(&p("hari ini jam 7 pagi")), (date(2026, 11, 6), Some(time(7, 0))));
    // Order does not matter.
    assert_eq!(due(&p("9am tomorrow")), (date(2026, 11, 7), Some(time(9, 0))));
}

#[test]
fn connectors_go_with_their_date_or_time() {
    let q = p("meeting with Sarah on monday at 3pm");
    assert_eq!(due(&q), (date(2026, 11, 9), Some(time(15, 0))));
    assert_eq!(q.title, "meeting with Sarah");
    assert_eq!(token_texts(&q), ["on monday", "at 3pm"]);
    let q = p("kumpul pada hari sabtu");
    assert_eq!(q.title, "kumpul");
    assert_eq!(due(&q).0, date(2026, 11, 7));
    // Without a date after it, a connector stays.
    assert_eq!(p("look at this").title, "look at this");
}

// ---- Repeats ----------------------------------------------------------------------------

#[test]
fn setiap_hari_tiap_hari_every_day_repeat_daily_from_today() {
    for input in [
        "minum vitamin setiap hari",
        "minum vitamin tiap hari",
        "minum vitamin every day",
        "minum vitamin daily",
    ] {
        let q = p(input);
        assert_eq!(q.repeat, Repeat::Daily, "{input}");
        assert_eq!(q.title, "minum vitamin", "{input}");
        assert_eq!(due(&q), (date(2026, 11, 6), None), "{input}");
        assert_eq!(kinds(&q), [TokenKind::Repeat], "{input}");
    }
}

#[test]
fn daily_with_a_passed_time_starts_tomorrow() {
    let q = p("olahraga tiap hari jam 7 pagi");
    assert_eq!(q.repeat, Repeat::Daily);
    assert_eq!(due(&q), (date(2026, 11, 7), Some(time(7, 0))));
    assert_eq!(q.title, "olahraga");
}

#[test]
fn tiap_senin_and_every_monday_repeat_weekly() {
    let q = p("buang sampah tiap senin");
    assert_eq!(q.repeat, Repeat::Weekly(Weekday::Monday));
    assert_eq!(due(&q).0, date(2026, 11, 9));
    assert_eq!(q.title, "buang sampah");
    assert_eq!(token_texts(&q), ["tiap senin"]);
    let q = p("team sync every monday 9am");
    assert_eq!(q.repeat, Repeat::Weekly(Weekday::Monday));
    assert_eq!(due(&q), (date(2026, 11, 9), Some(time(9, 0))));
    assert_eq!(p("setiap hari kamis").repeat, Repeat::Weekly(Weekday::Thursday));
}

#[test]
fn a_weekly_repeat_on_todays_weekday_starts_today_unless_the_time_passed() {
    let q = p("jumatan every friday");
    assert_eq!(q.repeat, Repeat::Weekly(Weekday::Friday));
    assert_eq!(due(&q).0, date(2026, 11, 6));
    assert_eq!(due(&p("tiap jumat jam 12")).0, date(2026, 11, 6));
    assert_eq!(due(&p("tiap jumat jam 8 pagi")).0, date(2026, 11, 13));
}

#[test]
fn an_explicit_date_sets_the_first_occurrence_of_a_repeat() {
    let q = p("bayar tagihan tiap hari mulai tgl 15");
    assert_eq!(q.repeat, Repeat::Daily);
    assert_eq!(due(&q).0, date(2026, 11, 15));
}

// ---- Lists, folders, people ---------------------------------------------------------------

#[test]
fn list_matches_case_insensitively_and_by_unique_prefix() {
    assert_eq!(p("x #BELANJA").list.as_deref(), Some("Belanja"));
    assert_eq!(p("x #bel").list.as_deref(), Some("Belanja"));
    assert_eq!(p("x #tag").list.as_deref(), Some("Tagihan"));
}

#[test]
fn an_ambiguous_or_unknown_list_stays_in_the_title() {
    // "#a" could be Admin or Archive.
    let q = p("cek #a");
    assert_eq!(q.list, None);
    assert_eq!(q.title, "cek #a");
    let q = p("fix bug #42 besok");
    assert_eq!(q.list, None);
    assert_eq!(q.title, "fix bug #42");
    assert_eq!(p("x #").list, None);
}

#[test]
fn an_exact_name_beats_a_longer_one_with_the_same_prefix() {
    let k = Known { lists: vec!["Work".into(), "Workout".into()], ..Known::default() };
    let q = parse("x #work", now_at(2026, 11, 6, 10, 0), &k);
    assert_eq!(q.list.as_deref(), Some("Work"));
    assert_eq!(parse("x #worko", now_at(2026, 11, 6, 10, 0), &k).list.as_deref(), Some("Workout"));
}

#[test]
fn folder_paths_match_by_segment() {
    assert_eq!(p("x /work").folder_path.as_deref(), Some("Work"));
    assert_eq!(p("x /Family/rumah").folder_path.as_deref(), Some("Family/Rumah"));
    assert_eq!(p("x /wo/client-b").folder_path.as_deref(), Some("Work/Client B"));
    assert_eq!(p("x /work/ad").folder_path.as_deref(), Some("Work/Admin"));
    // "client" fits both Client A and Client B.
    assert_eq!(p("x /work/client").folder_path, None);
    assert_eq!(p("x /nowhere").folder_path, None);
}

#[test]
fn people_match_case_insensitively_and_by_prefix() {
    assert_eq!(p("x @IBU").assignee.as_deref(), Some("Ibu"));
    assert_eq!(p("x @dew").assignee.as_deref(), Some("Dewi Lestari"));
    assert_eq!(p("x @dewi-lestari").assignee.as_deref(), Some("Dewi Lestari"));
    // "@d" could be Dimas or Dewi.
    let q = p("email @d");
    assert_eq!(q.assignee, None);
    assert_eq!(q.title, "email @d");
}

#[test]
fn everything_at_once() {
    let input = "Gas elpiji 3 kg besok jam 9 pagi #belanja /family/rumah @ibu";
    let q = p(input);
    assert_eq!(q.title, "Gas elpiji 3 kg");
    assert_eq!(due(&q), (date(2026, 11, 7), Some(time(9, 0))));
    assert_eq!(q.list.as_deref(), Some("Belanja"));
    assert_eq!(q.folder_path.as_deref(), Some("Family/Rumah"));
    assert_eq!(q.assignee.as_deref(), Some("Ibu"));
    assert_eq!(
        kinds(&q),
        [TokenKind::Date, TokenKind::Time, TokenKind::List, TokenKind::Folder, TokenKind::Person]
    );
    let starts: Vec<usize> = q.tokens.iter().map(|t| t.range.start).collect();
    assert!(starts.windows(2).all(|w| w[0] < w[1]));
    for t in &q.tokens {
        assert_eq!(&input[t.range.clone()], t.text);
    }
}

// ---- False positives ------------------------------------------------------------------------

#[test]
fn jam_dinding_is_not_a_time() {
    let q = p("beli jam dinding");
    assert_eq!(q.due, None);
    assert_eq!(q.title, "beli jam dinding");
    assert!(q.tokens.is_empty());
    assert_eq!(p("servis jam tangan besok").title, "servis jam tangan");
}

#[test]
fn a_weekday_inside_a_longer_word_is_not_a_date() {
    for input in
        ["daftar kesenian", "seninnya libur", "kamisol baru", "sunday-school notes", "fridays", "sabtuan"]
    {
        let q = p(input);
        assert_eq!(q.due, None, "{input}");
        assert_eq!(q.title, input, "{input}");
    }
}

#[test]
fn everyday_words_that_look_like_short_forms_stay_text() {
    // sen (cent), sel (cell), min (minus/admin) are not weekday abbreviations here.
    for input in ["ganti sel baterai", "tanya min", "uang sen"] {
        assert_eq!(p(input).due, None, "{input}");
    }
    assert_eq!(p("kerjain sat set").due, None);
}

#[test]
fn bare_numbers_are_not_times() {
    // "2 malam" is two nights at a hotel, not 2 at night.
    let q = p("booking hotel 2 malam");
    assert_eq!(q.due, None);
    assert_eq!(q.title, "booking hotel 2 malam");
    assert_eq!(p("beli 3 telur").due, None);
    assert_eq!(p("transfer 15.000").due, None);
    assert_eq!(p("timbang 1.5 kg").due, None);
}

#[test]
fn a_second_date_is_left_in_the_title() {
    let q = p("besok atau lusa");
    assert_eq!(due(&q).0, date(2026, 11, 7));
    assert_eq!(q.title, "atau lusa");
}

// ---- Title tidying -----------------------------------------------------------------------

#[test]
fn whitespace_is_tidied_and_punctuation_after_a_date_goes_with_it() {
    let q = p("  beli   galon\tbesok,  ke warung ");
    assert_eq!(q.title, "beli galon ke warung");
    assert_eq!(token_texts(&q), ["besok"]);
    assert_eq!(p("call mom tomorrow.").title, "call mom");
}

#[test]
fn plain_text_passes_through() {
    let q = p("Beli kado untuk Ayah");
    assert_eq!(q.title, "Beli kado untuk Ayah");
    assert_eq!(q.due, None);
    assert_eq!(q.repeat, Repeat::None);
    assert!(q.tokens.is_empty());
    assert_eq!(p("").title, "");
}

#[test]
fn non_ascii_text_keeps_valid_byte_ranges() {
    let input = "kué ultah 🎂 besok @dimas";
    let q = p(input);
    assert_eq!(q.title, "kué ultah 🎂");
    for t in &q.tokens {
        assert_eq!(&input[t.range.clone()], t.text);
    }
}
