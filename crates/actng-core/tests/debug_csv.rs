use actng_core::import::read_entries;

#[test]
fn test_user_csv() {
    let csv = "IBAN;Booked At;Text;Credit/Debit Amount;Balance;Valuta Date
CH3580808008146396840;2025-09-08 00:00:00.0;Achat siandora gmbh 05.09.2025, 20:24, No carte Visa Debit 427347xxxxxx8189;-251;11274.78;2025-09-08 00:00:00.0
CH3580808008146396840;2025-09-08 00:00:00.0;Achat HOTEL LA VETTA 04.09.2025, 08:32, No carte Visa Debit 427347xxxxxx8189 EUR 253.04, taux de change 0.9515;-243.78;11031;2025-09-08 00:00:00.0
;;inclus taxe pour achat à l'étranger CHF 3.01;;;
CH3580808008146396840;2025-09-08 00:00:00.0;Achat Zenhäusern Frères SA 06.09.2025, 12:38, No carte Visa Debit 427347xxxxxx8189;-14.3;11016.7;2025-09-08 00:00:00.0";
    
    let import = read_entries(csv.as_bytes(), None).expect("Import failed");
    println!("Profile: {:?}", import.profile);
    for (i, entry) in import.entries.iter().enumerate() {
        println!("Entry {}: date={:?}, desc={}, amount={:?}", i, entry.date, entry.description, entry.amount);
    }
    assert!(import.entries[0].date.is_some(), "First entry date should be parsed");
}
