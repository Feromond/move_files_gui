#![windows_subsystem = "windows"]

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use std::path::Path;
use std::sync::Arc;

use eframe::egui;
use eframe::egui::IconData;
use rfd::FileDialog;
use walkdir::WalkDir;

#[derive(PartialEq, Eq, Clone, Copy)]
enum InputType {
    File,
    Directory,
}

struct MyApp {
    input_path: String,
    /// Comma-separated list of file extensions (e.g., "pdf, jpg, png")
    extensions: String,
    output_path: String,
    input_type: InputType,
    log: String,
    /// Receiver for log messages coming from the background thread.
    log_rx: Option<mpsc::Receiver<String>>,
    /// Flag indicating if the move operation is running.
    is_moving: bool,
}

impl Default for MyApp {
    fn default() -> Self {
        Self {
            input_path: String::new(),
            extensions: String::new(),
            output_path: String::new(),
            input_type: InputType::Directory, // usually this will probably be a folder
            log: String::new(),
            log_rx: None,
            is_moving: false,
        }
    }
}

/// This function runs in a background thread. It recursively scans the input path
/// and moves all files with the specified extensions to the output folder,
/// sending progress messages back via the provided channel.
/// If the extensions string is empty, then every file is moved.
fn move_files_thread(
    input_path: String,
    output_path: String,
    extensions: String,
    input_type: InputType,
    sender: mpsc::Sender<String>,
) -> Result<(), Box<dyn Error>> {
    let output_dir = PathBuf::from(&output_path);
    fs::create_dir_all(&output_dir)?;
    
    // Parse the extensions string into a vector of normalized (lowercase, without dot) extensions.
    // If the user leaves this field blank, filter_exts will be empty.
    let filter_exts: Vec<String> = extensions
        .split(',')
        .map(|s| s.trim().trim_start_matches('.').to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    if input_type == InputType::Directory {
        let input_dir = PathBuf::from(&input_path);
        if !input_dir.is_dir() {
            let _ = sender.send(format!("{} is not a valid directory.\n", input_dir.display()));
            return Err(format!("{} is not a valid directory.", input_dir.display()).into());
        }
        // Walk the directory recursively.
        for entry in WalkDir::new(&input_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let file_path = entry.path();
            // Determine if the file should be moved:
            // - If filter_exts is empty, move every file.
            // - Otherwise, only move files whose extension (in lowercase) is in filter_exts.
            let should_move = if filter_exts.is_empty() {
                true
            } else if let Some(ext) = file_path.extension().and_then(|s| s.to_str()) {
                filter_exts.contains(&ext.to_lowercase())
            } else {
                false
            };

            if should_move {
                // Determine the output file path using the original file name.
                if let Some(file_name) = file_path.file_name() {
                    let mut dest_path = output_dir.join(file_name);
                    // If a file with the same name exists in the output, add a counter to avoid collision.
                    let mut counter = 1;
                    while dest_path.exists() {
                        let stem = file_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("file");
                        let new_name = if let Some(extension) = file_path.extension().and_then(|s| s.to_str()) {
                            format!("{}_{}.{}", stem, counter, extension)
                        } else {
                            format!("{}_{}", stem, counter)
                        };
                        dest_path = output_dir.join(new_name);
                        counter += 1;
                    }
                    // Attempt to move (rename) the file.
                    match fs::rename(file_path, &dest_path) {
                        Ok(_) => {
                            let _ = sender.send(format!(
                                "Moved: {} -> {}\n",
                                file_path.display(),
                                dest_path.display()
                            ));
                        }
                        Err(e) => {
                            let _ = sender.send(format!(
                                "Error moving {}: {}\n",
                                file_path.display(),
                                e
                            ));
                        }
                    }
                } else {
                    let _ = sender.send(format!(
                        "Warning: Skipping file with invalid name: {}\n",
                        file_path.display()
                    ));
                }
            }
        }
    } else {
        // Input is a single file.
        let file_path = PathBuf::from(&input_path);
        if !file_path.is_file() {
            let _ = sender.send(format!("{} is not a valid file.\n", file_path.display()));
            return Err(format!("{} is not a valid file.", file_path.display()).into());
        }
        let should_move = if filter_exts.is_empty() {
            true
        } else if let Some(ext) = file_path.extension().and_then(|s| s.to_str()) {
            filter_exts.contains(&ext.to_lowercase())
        } else {
            false
        };
        if should_move {
            if let Some(file_name) = file_path.file_name() {
                let mut dest_path = output_dir.join(file_name);
                let mut counter = 1;
                while dest_path.exists() {
                    let stem = file_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("file");
                    let new_name = if let Some(extension) = file_path.extension().and_then(|s| s.to_str()) {
                        format!("{}_{}.{}", stem, counter, extension)
                    } else {
                        format!("{}_{}", stem, counter)
                    };
                    dest_path = output_dir.join(new_name);
                    counter += 1;
                }
                match fs::rename(&file_path, &dest_path) {
                    Ok(_) => {
                        let _ = sender.send(format!(
                            "Moved: {} -> {}\n",
                            file_path.display(),
                            dest_path.display()
                        ));
                    }
                    Err(e) => {
                        let _ = sender.send(format!(
                            "Error moving {}: {}\n",
                            file_path.display(),
                            e
                        ));
                    }
                }
            } else {
                let _ = sender.send(format!(
                    "Warning: Skipping file with invalid name: {}\n",
                    file_path.display()
                ));
            }
        }
    }
    let _ = sender.send("Moving completed successfully.\n".to_string());
    Ok(())
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain any log messages coming from the background thread.
        if let Some(rx) = &self.log_rx {
            loop {
                match rx.try_recv() {
                    Ok(msg) => self.log.push_str(&msg),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.is_moving = false;
                        self.log_rx = None;
                        break;
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("File Mover");

            // Input type selection.
            ui.horizontal(|ui| {
                ui.label("Input Type:");
                ui.radio_value(&mut self.input_type, InputType::File, "File");
                ui.radio_value(&mut self.input_type, InputType::Directory, "Directory");
            });

            // Input path.
            ui.horizontal(|ui| {
                ui.label("Input Path:");
                ui.text_edit_singleline(&mut self.input_path);
                if ui.button("Browse").clicked() {
                    let selected = if self.input_type == InputType::File {
                        FileDialog::new().pick_file()
                    } else {
                        FileDialog::new().pick_folder()
                    };
                    if let Some(path) = selected {
                        self.input_path = path.display().to_string();
                    }
                }
            });

            // Extensions field.
            ui.horizontal(|ui| {
                ui.label("Extensions (comma-separated, e.g., pdf, jpg, png):");
                ui.text_edit_singleline(&mut self.extensions);
            });

            // Output directory.
            ui.horizontal(|ui| {
                ui.label("Output Directory:");
                ui.text_edit_singleline(&mut self.output_path);
                if ui.button("Browse").clicked() {
                    if let Some(path) = FileDialog::new().pick_folder() {
                        self.output_path = path.display().to_string();
                    }
                }
            });

            // Button to start moving files.
            if ui.button("Move Files").clicked() && !self.is_moving {
                self.log.clear();
                let input_path = self.input_path.clone();
                let output_path = self.output_path.clone();
                let extensions = self.extensions.clone();
                let input_type = self.input_type;
                let (tx, rx) = mpsc::channel::<String>();
                self.log_rx = Some(rx);
                self.is_moving = true;
                thread::spawn(move || {
                    let _ = move_files_thread(input_path, output_path, extensions, input_type, tx);
                });
            }

            ui.separator();

            // Log output in a scrollable area that sticks to the bottom.
            ui.label("Log:");
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.log)
                            .desired_rows(20)
                            .desired_width(600.0),
                    );
                });
        });
    }
}

fn main() {
    let icon_path = Path::new("icon.ico");

    let icon_data = if icon_path.exists() {
        let image = image::open(icon_path)
            .expect("Failed to open icon.ico")
            .to_rgba8();
        let (width, height) = image.dimensions();
        IconData {
            rgba: image.into_raw(),
            width,
            height,
        }
    } else {
        // Fallback: use a transparent 32x32 icon.
        IconData {
            rgba: vec![0; 32 * 32 * 4],
            width: 32,
            height: 32,
        }
    };

    let mut native_options = eframe::NativeOptions::default();
    // Set the icon via the viewport's icon field.
    native_options.viewport.icon = Some(Arc::new(icon_data));

    let _ = eframe::run_native(
        "File Mover",
        native_options,
        Box::new(|_cc| Ok(Box::new(MyApp::default()))),
    );
}