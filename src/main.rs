use std::{
    collections::HashMap,
    fs,
    io::{self, ErrorKind},
    net::UdpSocket,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

#[derive(Debug)]
struct SensorBatch {
    sensors: Vec<SensorData>,
    timestamp: f64,
}

#[derive(Debug)]
struct SensorData {
    sensor: u8,
    x: f64,
    y: f64,
    z: f64,
}

fn parse_sensordata(parts: &[&str]) -> Option<SensorData> {
    Some(SensorData {
        sensor: str::parse(&parts[0].trim()).ok()?,
        x: str::parse(&parts[1].trim()).ok()?,
        y: str::parse(&parts[2].trim()).ok()?,
        z: str::parse(&parts[3].trim()).ok()?,
    })
}

fn parse_sensor_batch(total: &[u8]) -> Option<SensorBatch> {
    let total_str = String::from_utf8_lossy(total);
    let parts: Vec<_> = total_str.split(|b| b == ',').collect();
    let time_str = parts.get(0)?;
    let time: f64 = str::parse(time_str).ok()?;

    let mut ret = SensorBatch {
        sensors: vec![],
        timestamp: time,
    };

    if (parts.len() - 1) % 4 != 0 {
        println!("message wrong length!");
        return None;
    }

    for sensor_i in 0..(parts.len() - 1) / 4 {
        let subparts = &parts[sensor_i * 4 + 1..(sensor_i + 1) * 4 + 1];
        ret.sensors.push(parse_sensordata(subparts)?);
    }

    Some(ret)
}

fn get_next_data(socket: &UdpSocket) -> Result<Option<SensorBatch>, io::Error> {
    let mut buf = [0; 4096];
    let len = match socket.recv(&mut buf) {
        Ok(v) => v,
        Err(e) => {
            if e.kind() == ErrorKind::Interrupted {
                println!("Socket read was interrupted. This is probably ok.");
                return Err(e);
            }
            eprintln!("Uncaught error: {:?}, {}", e, e);
            return Err(e);
        }
    };

    let ret = parse_sensor_batch(&buf[..len]);
    Ok(ret)
}

fn save_file(csv: &Vec<Vec<String>>) {
    let mut csv_bytes = vec![];
    for row in 0..csv[0].len() {
        for column in 0..csv.len() {
            if row >= csv[column].len() {
                csv_bytes.push(',' as u8);
                continue;
            }
            let cell_bytes = csv[column][row].as_bytes();
            csv_bytes.extend_from_slice(cell_bytes);
            csv_bytes.push(',' as u8);
        }
        csv_bytes.push('\n' as u8);
    }

    fs::write("output.csv", csv_bytes).unwrap();
}

fn main() {
    let socket = UdpSocket::bind("0.0.0.0:5555").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    println!("Enter the devices in the following format: <device #>:<device name>,:");
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();

    let mut device_map = HashMap::new();
    line.split(',').for_each(|part| {
        let subparts: Vec<&str> = part.split(':').collect();
        let num: u8 = str::parse(subparts.get(0).expect("Invalid formatting: not enough parts between commas (do not use a trailing comma)").trim()).expect(&format!("Invalid formatting: {} is not an integer in [0,255]", subparts[0]));
        let name = subparts.get(1).expect(&format!("Invalid formatting: name not supplied for device number {}", num)).trim();
        device_map.insert(num, (name.to_string(), 0));
    });

    let output_csv_interior_mut = Arc::new(Mutex::new(vec![]));
    let mut output_csv = output_csv_interior_mut.lock().unwrap();
    let mut column = 0;
    for (num, name) in &mut device_map {
        output_csv.push(vec!["Time (s)".to_string()]);
        output_csv.push(vec![format!("{}: X ({})", name.0, num)]);
        output_csv.push(vec![format!("{}: Y ({})", name.0, num)]);
        output_csv.push(vec![format!("{}: Z ({})", name.0, num)]);
        output_csv.push(vec![String::new()]);
        name.1 = column;
        column += 5;
    }
    drop(output_csv);

    let ctrl_c_output_csv = output_csv_interior_mut.clone();
    let quit = Arc::new(AtomicBool::new(false));
    let (file_read_done_tx, file_read_done_rx) = mpsc::channel();
    let ctrl_c_quit = quit.clone();
    ctrlc::set_handler(move || {
        save_file(&ctrl_c_output_csv.lock().unwrap());
        println!("Quitting... This should not take longer than 5 seconds.");
        ctrl_c_quit.store(true, Ordering::SeqCst);
        file_read_done_tx.send(()).unwrap();
    })
    .expect("Error setting Ctrl-C handler");

    println!("should be collecting data...");
    while !quit.load(Ordering::SeqCst) {
        let res = match get_next_data(&socket) {
            Ok(v) => v,
            Err(_) => break,
        };
        let Some(batch) = res else {
            continue;
        };
        let mut output_csv = output_csv_interior_mut.lock().unwrap(); //this is gross but setting up a channel would be even more incomprehensible and it's not like it needs to be super high-performance

        for sensor_data in &batch.sensors {
            let column = device_map
                .get(&sensor_data.sensor)
                .expect("Found unspecified sensor. Please make sure to enter all devices.")
                .1;
            output_csv[column].push(format!("{}", batch.timestamp));
            output_csv[column + 1].push(format!("{}", sensor_data.x));
            output_csv[column + 2].push(format!("{}", sensor_data.y));
            output_csv[column + 3].push(format!("{}", sensor_data.z));
        }
    }
    file_read_done_rx.recv().unwrap();
}
