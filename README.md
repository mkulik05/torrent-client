# Torrent client
## About
It's a simple torrent client written for educational purposes.
Supported features:
- UDP and TCP torrent-trackers;
- Multifile torrents downloading;
- Tracking progress and statistics (speed, remaining time);
- Multiple downloads at the same time;
- Torrent pause;
- Automatic session save on close.


## Installation from the source


1. Make sure you have [rust language](https://www.rust-lang.org/tools/install) installed.
2. Clone the repository and enter the cloned folder`git clone https://github.com/mkulik05/torrent-client && cd torrent-client`
3. Run `cargo build --release`
4. Run `./target/release/torrent-client` to start client


## Working logic
### General scheme
For downloading, several threads are created. They form this structure:


![Threads scheme](https://i.imgur.com/7rJQzZ2.png)


First, a general thread ("Worker") is created, which immediately creates a ["Saver" thread](#saver) intended for saving downloaded parts and a thread for finding peers for the current torrent (see [this](#peer-search)).


MPSC (multi-producer-single-consumer) channels are used for communication with the threads to send messages. One channel is needed to pass verified peers to the "Worker" thread. For each such peer, a thread is created and assigned a task (see [this](#download-tasks) for more details). After this, the "Worker" thread waits until the peer is free (because a peer can only perform one task at a time) and then assigns a new task. MPSC is also used to transfer downloaded parts from the downloading threads to the "Saver" thread.


### Saver


The task of the "Saver" thread is to save the downloaded parts (when working with a torrent containing multiple files, it is also necessary to determine which file the downloaded part belongs to, as there may be several). Additionally, for each part, the downloaded chunks are tracked. When all chunks of a part are downloaded, a hash check is performed for that part. The "Saver" thread finishes its work once all parts are downloaded and their hashes are verified.


Chunk tracking is implemented using a hash table. The index of the part serves as the key, and the value is a mask of downloaded chunks (a byte array where each bit indicates the presence or absence of a chunk for this part), with the mask of the last value (the number of chunks may not be a multiple of 8).


For multi-file torrents, it is necessary to determine which files a segment of the specified length and initial offset belongs to. To do this, a cumulative array of file sizes is created based on the file sizes, where the first element is zero and each subsequent element is the sum of the previous element and the size of the file at the previous index. The final offset of the segment is then calculated, and for both offsets, the index of the first element in the array that is greater than the offset is found. Based on these indices, the range of files that the segment of bytes covers is determined.


### Peer search
A separate thread is created for finding peers. Within it, a thread is created for each tracker from the torrent file, which sends a request to the tracker to obtain a list of peers. After that, each peer is checked in a separate thread (by establishing a TCP connection and sending a handshake).


### Download tasks
Two queues are used for distributing download tasks. The first queue is for parts, and the second is for chunks.


The queue for parts is formed from the beginning, with tasks for each part added to it (each element contains the index of the part, the number of chunks, and the number of downloaded chunks, initially set to zero).


The second queue is formed gradually, with a limit on its maximum size. It contains the index of the part, the range of chunks to download, and a flag indicating whether this range includes the last chunk (its size will differ in this case). When adding an element to the second queue, the first element from the parts queue is used. The number of downloaded chunks in it increases by the length of the range (fixed except for the last value). Once the number of downloaded chunks equals the total number of chunks, the element is removed from the first queue.


- If errors occur while downloading chunks, the task is returned to the second queue.
- If the hash of a part does not match, the task for that part is returned to the first queue.


### Saving download state
To implement functionality such as pausing, and saving downloading progress between app's runs, the following information is saved for each torrent in the app download list:
- Parsed torrent file;
- Both task queues;
- The path where the result is saved;
- The number of downloaded parts;
- The status of the torrent (paused, downloading, completed).