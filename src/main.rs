use pixels::{Pixels, SurfaceTexture};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};
use winit::window::WindowButtons;
use std::sync::Arc;
use std::collections::HashMap;
use sysinfo::{System, ProcessesToUpdate};

mod world;
use world::World;

const WIDTH: u32 = 480;
const HEIGHT: u32 = 540;
const FONT_DATA: &[u8] = include_bytes!("../assets/font.ttf");

#[derive(Clone, Copy, PartialEq)]
enum SortBy { Name, CPU, RAM, PPID, PID }

// The lifetime 'a indicates the Pixels object borrows from the window
struct TaskManager {
    window: Option<Arc<Window>>,
    pixels: Option<Pixels<'static>>,
    world: World,
    sys: System,
    scroll_offset: usize, // New: track scroll position
    is_dragging: bool,
    last_mouse_y: f64,
    last_update: std::time::Instant,
    processes: Vec<(std::path::PathBuf, (u64, f32, String, u32, u32))>,
    global_vram_used: u64,
    global_vram_total: u64,
    nvml: Option<nvml_wrapper::Nvml>,
    global_rx: u64, // Received
    global_tx: u64, // Transmitted
    display_rx: f64,
    display_tx: f64,
    last_net_update: std::time::Instant,
    networks: sysinfo::Networks,
    sort_by: SortBy,
    sort_ascending: bool, // New: track sort direction
    last_mouse_x: f64,
}

impl TaskManager {
    fn new() -> Self {
        Self {
            window: None,
            pixels: None,
            world: World::new(WIDTH as usize, HEIGHT as usize, FONT_DATA),
            sys: System::new_all(),
            scroll_offset: 0,
            is_dragging: false,
            last_mouse_y: 0.0,
            last_update: std::time::Instant::now(),
            processes: Vec::new(),
            global_vram_used: 0,
            global_vram_total: 0,
            nvml: nvml_wrapper::Nvml::init().ok(),
            global_rx: 0,
            global_tx: 0,
            display_rx: 0.0,
            display_tx: 0.0,
            last_net_update: std::time::Instant::now(),
            networks: sysinfo::Networks::new_with_refreshed_list(),
            sort_by: SortBy::RAM,
            sort_ascending: false,
            last_mouse_x: 0.0,
        }
    }

    fn get_total_scrollable_items(&self) -> usize {
        self.processes.len().saturating_sub(15)
    }

    fn calculate_thumb_y(&self) -> u32 {
        let track_y = 70.0;
        let track_h = 375.0;
        let thumb_h = 50.0;
        let max_offset = self.get_total_scrollable_items();

        if max_offset == 0 { return track_y as u32; }

        let ratio = self.scroll_offset as f32 / max_offset as f32;
        (track_y + (ratio * (track_h - thumb_h))) as u32
    }
}

impl ApplicationHandler for TaskManager {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes = winit::window::Window::default_attributes()
            .with_title("Task Manager")
            .with_inner_size(winit::dpi::LogicalSize::new(WIDTH, HEIGHT))
            .with_resizable(false)
            .with_enabled_buttons(WindowButtons::MINIMIZE | WindowButtons::CLOSE)
            .with_min_inner_size(winit::dpi::LogicalSize::new(WIDTH, HEIGHT))
            .with_max_inner_size(winit::dpi::LogicalSize::new(WIDTH, HEIGHT));

        // Explicitly set the app_id / WM_CLASS to "taskman" to avoid aliases and title fallback
        #[cfg(target_os = "linux")]
        let window_attributes = {
            use winit::platform::wayland::WindowAttributesExtWayland;
            use winit::platform::x11::WindowAttributesExtX11;
            let wa = WindowAttributesExtWayland::with_name(window_attributes, "taskman", "taskman");
            WindowAttributesExtX11::with_name(wa, "taskman", "taskman")
        };

        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

        // By moving window creation here and using a static-safe approach, 
        // we can initialize Pixels safely.
        let window_clone = window.clone();
        let surface_texture = SurfaceTexture::new(WIDTH, HEIGHT, &*window_clone);
        let pixels = Pixels::new(WIDTH, HEIGHT, surface_texture).expect("Pixels error");

        self.window = Some(window);
        // We use unsafe to cast to 'static because the window is owned 
        // by the TaskManager (which lives for the program's duration)
        self.pixels = Some(unsafe { std::mem::transmute::<Pixels<'_>, Pixels<'static>>(pixels) });
    }
    
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
            match event {
                // Window close
                WindowEvent::CloseRequested => {
                    event_loop.exit();
                }
                
				// Window resize
                WindowEvent::Resized(size) => {
                    // Enforce fixed dimensions to block maximizing/stretching
                    if size.width != WIDTH || size.height != HEIGHT {
                        if let Some(window) = &self.window {
                            window.request_inner_size(winit::dpi::LogicalSize::new(WIDTH, HEIGHT));
                        }
                    } else if let Some(pixels) = &mut self.pixels {
                        if size.width > 0 && size.height > 0 {
                            pixels.resize_surface(size.width, size.height).unwrap();
                            pixels.resize_buffer(size.width, size.height).unwrap();
                        }
                    }
                }
                // Handle Scrolling
                WindowEvent::CursorMoved { position, .. } => {

                    self.last_mouse_x = position.x;
                    self.last_mouse_y = position.y; // Track Y for clicking
                    
                    if self.is_dragging {
                        let track_y = 70.0; // Ensure this matches your drawing Y coordinate
                        let track_h = 375.0;
                        let thumb_h = 50.0;
                        
                        // Constrain mouse to the track
                        let mouse_y = position.y as f32;
                        let relative_y = (mouse_y - track_y - (thumb_h / 2.0)).clamp(0.0, track_h - thumb_h);
                        
                        let max_offset = self.get_total_scrollable_items();
                        let percentage = relative_y / (track_h - thumb_h);
                        
                        self.scroll_offset = (percentage * max_offset as f32).round() as usize;
                    }
                }

                WindowEvent::MouseInput { state, button, .. } => {
                    if button == winit::event::MouseButton::Left {
                        if state == winit::event::ElementState::Pressed {
                            
                            // 1. Header Click Test (Sort)
                            if self.last_mouse_y >= 40.0 && self.last_mouse_y <= 50.0 {
                                let new_sort = if self.last_mouse_x >= 0.0 && self.last_mouse_x < 60.0 { Some(SortBy::Name) }
                                    else if self.last_mouse_x >= 200.0 && self.last_mouse_x < 250.0 { Some(SortBy::PID) }
                                    else if self.last_mouse_x >= 250.0 && self.last_mouse_x < 300.0 { Some(SortBy::PPID) }
                                    else if self.last_mouse_x >= 310.0 && self.last_mouse_x < 360.0 { Some(SortBy::CPU) }
                                    else if self.last_mouse_x >= 390.0 && self.last_mouse_x < 450.0 { Some(SortBy::RAM) }
                                    else { None };

                                if let Some(new_sort) = new_sort {
                                    match self.sort_by {
                                        SortBy::Name if matches!(new_sort, SortBy::Name) => self.sort_ascending = !self.sort_ascending,
                                        SortBy::PID if matches!(new_sort, SortBy::PID) => self.sort_ascending = !self.sort_ascending,
                                        SortBy::PPID if matches!(new_sort, SortBy::PPID) => self.sort_ascending = !self.sort_ascending,
                                        SortBy::CPU if matches!(new_sort, SortBy::CPU) => self.sort_ascending = !self.sort_ascending,
                                        SortBy::RAM if matches!(new_sort, SortBy::RAM) => self.sort_ascending = !self.sort_ascending,
                                        _ => {
                                            self.sort_by = new_sort;
                                            self.sort_ascending = false; // Default descending for new sort
                                        }
                                    }
                                }
                            }
                            // 2. Scrollbar Click Test (Drag)
                            else {
                                let track_h = 375.0;
                                let thumb_h = 50.0;
                                let processes_len = self.sys.processes().len().max(1);
                                let thumb_y = 60.0 + ((self.scroll_offset as f32 / processes_len as f32) * (track_h - thumb_h));
                
                                if self.last_mouse_y >= thumb_y as f64 && self.last_mouse_y <= (thumb_y + thumb_h) as f64 {
                                    self.is_dragging = true;
                                }
                            }
                        } else {
                            // Mouse Released
                            self.is_dragging = false;
                        }
                    }
                }         

                
                WindowEvent::MouseWheel { delta, .. } => {
                    let scroll_y = match delta {
                        winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                        _ => 0.0,
                    };
                    
                    if scroll_y > 0.0 && self.scroll_offset > 0 {
                        self.scroll_offset -= 1;
                    } else if scroll_y < 0.0 {
                        self.scroll_offset += 1;
                    }
                }
                
                WindowEvent::RedrawRequested => {
                    // 1. Refresh data only if 3 second has passed
                    if self.last_update.elapsed().as_secs() >= 1 {
                        self.sys.refresh_processes(ProcessesToUpdate::All, true);
                        self.sys.refresh_memory();

                        // NVML
                        if let Some(nvml) = &self.nvml {
                            if let Ok(device) = nvml.device_by_index(0) {
                                if let Ok(mem_info) = device.memory_info() {
                                    self.global_vram_used = mem_info.used;
                                    self.global_vram_total = mem_info.total;
                                }
                            }
                        }
                        self.last_update = std::time::Instant::now();

                        // Refresh network data
                        // 1. Refresh network data
                        self.networks.refresh(true); 

                        // 2. Aggregate current values
                        let mut total_rx: u64 = 0;
                        let mut total_tx: u64 = 0;
                        for (_, data) in &self.networks {
                            total_rx += data.received();
                            total_tx += data.transmitted();
                        }

                        // 3. Calculate speed
                        let elapsed = self.last_net_update.elapsed().as_secs_f64();

                        // If elapsed is near 0, we avoid division by zero
                        if elapsed > 0.1 {
                            self.display_rx = (total_rx.saturating_sub(self.global_rx)) as f64 / elapsed;
                            self.display_tx = (total_tx.saturating_sub(self.global_tx)) as f64 / elapsed;
                        }

                        // 4. IMPORTANT: Only update globals after calculation
                        self.global_rx = total_rx;
                        self.global_tx = total_tx;
                        self.last_net_update = std::time::Instant::now();
                        
                    }

                    // 2. Group processes by name
                    // Use PathBuf as the key, and a 5-element tuple as the value
                    let mut process_groups: HashMap<std::path::PathBuf, (u64, f32, String, u32, u32)> = HashMap::new();

                    let core_count = (self.sys.cpus().len() as f32).max(1.0); // Get the number of cores

                    for (pid, proc) in self.sys.processes() {
                        if let Some(exe_path) = proc.exe() {
                            let name = proc.name().to_string_lossy().to_string();
                            let ppid = proc.parent().map(|p| p.as_u32()).unwrap_or(0); // Get PPID
                            let pid_val = pid.as_u32();

                            let entry = process_groups.entry(exe_path.to_path_buf())
                                .or_insert((0, 0.0, name, ppid, pid_val)); // (RAM, CPU, Name, PPID, PID) - 5 elements

                            entry.0 = entry.0.max(proc.memory());

                            let normalized_cpu = proc.cpu_usage() / core_count;
                            entry.1 = entry.1.max(normalized_cpu);
                            // Keep the smallest PID for the group (or logic of your choice)
                            entry.4 = entry.4.min(pid_val);
                        }
                    }

                    // 2. Convert once and store directly into your struct field
                    self.processes = process_groups.into_iter().collect();

                    // 3. Sort the struct field directly
                    match (self.sort_by, self.sort_ascending) {
                        (SortBy::Name, true) => self.processes.sort_by(|a, b| a.1.2.cmp(&b.1.2)),
                        (SortBy::Name, false) => self.processes.sort_by(|a, b| b.1.2.cmp(&a.1.2)),
                        (SortBy::CPU, true) => self.processes.sort_by(|a, b| a.1.1.partial_cmp(&b.1.1).unwrap_or(std::cmp::Ordering::Equal)),
                        (SortBy::CPU, false) => self.processes.sort_by(|a, b| b.1.1.partial_cmp(&a.1.1).unwrap_or(std::cmp::Ordering::Equal)),
                        (SortBy::PPID, true) => self.processes.sort_by(|a, b| a.1.3.cmp(&b.1.3)),
                        (SortBy::PPID, false) => self.processes.sort_by(|a, b| b.1.3.cmp(&a.1.3)),
                        (SortBy::PID, true) => self.processes.sort_by(|a, b| a.1.4.cmp(&b.1.4)),
                        (SortBy::PID, false) => self.processes.sort_by(|a, b| b.1.4.cmp(&a.1.4)),
                        (SortBy::RAM, true) => self.processes.sort_by(|a, b| a.1.0.cmp(&b.1.0)),
                        (SortBy::RAM, false) => self.processes.sort_by(|a, b| b.1.0.cmp(&a.1.0)),
                    }

                    if let Some(pixels) = &mut self.pixels {
                        let frame = pixels.frame_mut();
                        frame.fill(0);

                        // RAM Header
                        let total_ram = self.sys.total_memory();
                        let used_ram = self.sys.used_memory();
                        let mem_header = format!("RAM: {} / {} MiB", used_ram / 1024 / 1024, total_ram / 1024 / 1024);
                        self.world.draw_text(frame, &mem_header, 10, 500, 18.0, [0, 255, 255]);

                        // VRAM Header
                        let vram_header = format!(
                            "VRAM: {} / {} MiB", 
                            self.global_vram_used / 1024 / 1024, 
                            self.global_vram_total / 1024 / 1024
                        );
                        self.world.draw_text(frame, &vram_header, 250, 500, 18.0, [255, 0, 255]);
                        
                        // Column Titles
                        self.world.draw_text(frame, "NAME", 10, 50, 14.0, [255, 255, 0]);
                        self.world.draw_text(frame, "PID", 210, 50, 14.0, [255, 255, 0]);
                        self.world.draw_text(frame, "PPID", 260, 50, 14.0, [255, 255, 0]);
                        self.world.draw_text(frame, "CPU", 320, 50, 14.0, [255, 255, 0]);
                        self.world.draw_text(frame, "RAM", 400, 50, 14.0, [255, 255, 0]);

                        // Process Rows (Grey rectangles removed for clean look)
                        for (i, (path, (ram, cpu, name, ppid, pid))) in self.processes.iter().skip(self.scroll_offset).take(15).enumerate() {
                            let y = (80 + (i as u32 * 25)) as usize;
                            let display_name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or(std::borrow::Cow::Borrowed(name));
                        
                            // 1. Name
                            self.world.draw_text(frame, &display_name, 10, y, 12.0, [255, 255, 255]);

                            // 2. PID
                            self.world.draw_text(frame, &pid.to_string(), 210, y, 12.0, [200, 200, 200]);

                            // 3. PPID
                            self.world.draw_text(frame, &ppid.to_string(), 260, y, 12.0, [200, 200, 200]);
                            
                            // 4. CPU
                            self.world.draw_text(frame, &format!("{:.1}%", cpu), 320, y, 12.0, [0, 255, 0]);
                            
                            // 5. RAM
                            self.world.draw_text(frame, &format!("{}M", ram / 1024 / 1024), 400, y, 12.0, [200, 200, 200]);
                        }
                        // Scrollbar
                        // 1. Calculate the scrollable range
                        // If we show 15 items at a time, we only need to scroll if we have more than 15.
                        let visible_items = 15;
                        let total_items = self.processes.len();
                        let scrollable_range = if total_items > visible_items {
                            total_items - visible_items
                        } else {
                            0
                        };


                        // Network
                        let net_text = format!(
                            "NET: D:{:.1}KB/s U:{:.1}KB/s", 
                            self.display_rx / 1024.0, 
                            self.display_tx / 1024.0
                        );
                        
                        self.world.draw_text(frame, &net_text, 10, 520, 16.0, [100, 255, 100]);

                        // Refresh global CPU usage
                        self.sys.refresh_cpu_all(); 
                        let total_cpu = self.sys.global_cpu_usage();
                        
                        // Draw it
                        self.world.draw_text(frame, &format!("TOTAL CPU: {:.1}%", total_cpu), 10, 480, 18.0, [255, 255, 255]);
                        
                        // 2. Calculate Thumb Position
                        let track_y = 70.0;
                        let track_h = 375.0;
                        let thumb_h = 50.0;
                        
                        let thumb_y = if scrollable_range > 0 {
                            // Map the scroll_offset to the available track length
                            let ratio = self.scroll_offset as f32 / scrollable_range as f32;
                            track_y + (ratio * (track_h - thumb_h))
                        } else {
                            track_y // Thumb stays at top if there's nothing to scroll
                        };
                        
                        // 3. Draw

                        // Draw the scrollbar track
                        self.world.draw_rect(frame, 450, track_y as u32, 15, track_h as u32, [40, 40, 40, 255]); 
                        
                        // Draw the thumb (the moving part)
                        // Using thumb_y calculated in your existing code
                        self.world.draw_rect(frame, 450, thumb_y as u32, 15, thumb_h as u32, [100, 100, 100, 255]);
                        
                        pixels.render().unwrap();
                    }
                    
                    if let Some(window) = &self.window { window.request_redraw(); }
                }
                _ => (),
            }
        }
}

fn main() {
    // Check for version flags before starting the GUI event loop
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "--version" || args[1] == "-V") {
        println!("taskman {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let event_loop = EventLoop::new().unwrap();
    let mut app = TaskManager::new();
    event_loop.run_app(&mut app).unwrap();
}
