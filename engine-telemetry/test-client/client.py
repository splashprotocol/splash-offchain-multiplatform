import socket

UDP_IP = "127.0.0.1"
UDP_PORT = 8090
MESSAGE = b"""{"pair":["00","d3917756db639cf4a7dbc2a61684fe1fc99862e0da81708c2493c41f434154"],"executions":[{"id":["d3917756db639cf4a7dbc2a61684fe1fc99862e0da81708c2493c41f","CAT"],"version":"59811364865a45bc001dd81f8c7bf21bf74749f84d7a8a061505a38c14fec544#0","mean_price":[1,2],"removed_input":1,"added_output":2,"side":"Bid"}],"meta":{"mean_spot_price":[1,2]},"tx_hash":"8064bf12c840f8c5abd319359a31d181c5bec3237b903fa8577a081669638a08"}@"""

print("UDP target IP: %s" % UDP_IP)
print("UDP target port: %s" % UDP_PORT)
print("message: %s" % MESSAGE)

sock = socket.socket(socket.AF_INET, # Internet
                     socket.SOCK_STREAM) # UDP
sock.connect((UDP_IP, UDP_PORT))
sock.sendall(MESSAGE)

print("Done")